//! Build the one-screen cockpit from commit data + the survival collection.

use std::collections::{BTreeMap, HashMap};

use crate::git::Collection;
use crate::model::{Card, Cockpit, Commit, Heatmap, Intent, Me, RepoSurvival, Tone};
use crate::spark::{median, sparkline, weighted_median, wilson};
use crate::survival::{half_life, km_survival, sample_survival, survival_at};

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

/// Recency half-life: the calendar time over which a behavior's weight halves. Fixed,
/// NOT scaled by repo age — that is the whole point. It makes "how I've been working
/// lately" repo-age-independent and responsive within ~1 half-life (spike: ~21d reaches
/// halfway to a step change in ~4 weeks), instead of an all-history average an old repo
/// can never move. Decays by wall-clock, not commit count, so a burst of commits can't
/// age out real history. The 8-week sparkline carries the slower context.
pub const RECENCY_HALFLIFE_DAYS: f64 = 21.0;

/// A plain-language anchor table for the recency weighting (for `--explain` / the
/// report): a commit's pull on every figure, by age, vs one made today — the weight
/// halves every half-life. Built from the one constant so it can never drift. e.g.
/// "today 100% · 3wk 50% · 6wk 25% · 8wk 16% · 3mo 5% · 1yr ~0%".
pub fn recency_anchors() -> String {
    [
        ("today", 0.0),
        ("3wk", 21.0),
        ("6wk", 42.0),
        ("8wk", 56.0),
        ("3mo", 91.0),
        ("1yr", 365.0),
    ]
    .iter()
    .map(|(label, days)| {
        let w = 0.5_f64.powf(days / RECENCY_HALFLIFE_DAYS) * 100.0;
        if w >= 1.0 {
            format!("{label} {w:.0}%")
        } else {
            format!("{label} ~0%")
        }
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Calendar-time recency weight for a timestamp: 1.0 at the anchor (newest commit),
/// halving every [`RECENCY_HALFLIFE_DAYS`]. Future timestamps clamp to the anchor.
pub fn recency(ts: i64, anchor: i64) -> f64 {
    const LAMBDA: f64 = std::f64::consts::LN_2 / (RECENCY_HALFLIFE_DAYS * DAY as f64);
    (-LAMBDA * (anchor - ts).max(0) as f64).exp()
}

/// The recency lens as a reusable weight closure: `recency(ts, anchor)` by default, a flat
/// `1.0` under `--all`. Built once per anchor and threaded into every metric so the "one
/// lens, all surfaces reconcile" rule is structural rather than copy-pasted per call site.
pub fn recency_lens(all_time: bool, anchor: i64) -> impl Fn(i64) -> f64 + Copy {
    move |ts| if all_time { 1.0 } else { recency(ts, anchor) }
}

/// Recency weight for a whole week bucket `wk` weeks before the anchor (0 = current) —
/// the per-week form of [`recency`], same half-life. Used for flow/batch "typical".
fn week_decay(wk: i64) -> f64 {
    (-std::f64::consts::LN_2 * (wk as f64) * 7.0 / RECENCY_HALFLIFE_DAYS).exp()
}

/// Effective sample size of a weighted set, `(Σw)² / Σw²` — how many commits actually
/// back a recency-weighted rate (≈ the count when weights are flat, far fewer when a
/// handful of recent commits dominate). One definition, shared by every honesty band.
fn effective_n(sw: f64, sw2: f64) -> f64 {
    if sw2 > 0.0 {
        sw * sw / sw2
    } else {
        0.0
    }
}

fn hour_of(ts: i64) -> i64 {
    ts.rem_euclid(DAY) / 3600
}

/// 0 = Monday .. 6 = Sunday. (1970-01-01 was a Thursday = 3.)
fn weekday_mon0(ts: i64) -> i64 {
    (ts.div_euclid(DAY).rem_euclid(7) + 3).rem_euclid(7)
}

fn is_night(ts: i64) -> bool {
    !(6..20).contains(&hour_of(ts))
}

fn is_weekend(ts: i64) -> bool {
    weekday_mon0(ts) >= 5
}

/// Most-recent-first → week bucket (0 = current week).
fn week_bucket(ts: i64, anchor: i64) -> i64 {
    (anchor - ts).div_euclid(WEEK)
}

/// Local UTC offset in seconds via `date +%z` (zero-dep). None if unavailable.
/// Uses the *current* offset for all timestamps — DST ±1h doesn't move the
/// night/weekend classification meaningfully.
fn local_offset_secs() -> Option<i64> {
    let out = std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim();
    if s.len() < 5 {
        return None;
    }
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let hh: i64 = s[1..3].parse().ok()?;
    let mm: i64 = s[3..5].parse().ok()?;
    Some(sign * (hh * 3600 + mm * 60))
}

fn tz_label(offset: Option<i64>) -> String {
    match offset {
        None | Some(0) => "UTC".to_string(),
        Some(o) => {
            let sign = if o < 0 { '-' } else { '+' };
            let a = o.abs();
            let (h, m) = (a / 3600, (a % 3600) / 60);
            if m == 0 {
                format!("local, UTC{sign}{h}")
            } else {
                format!("local, UTC{sign}{h}:{m:02}")
            }
        }
    }
}

pub fn build_cockpit(
    commits: &[Commit],
    repos: &[(String, Collection)],
    branch: &str,
    me: Option<&Me>,
    as_of: Option<&str>,
) -> Cockpit {
    let anchor = commits.iter().map(|c| c.ts).max().unwrap_or(0);
    let min_ts = commits.iter().map(|c| c.ts).min().unwrap_or(anchor);
    let coverage_weeks = ((anchor - min_ts).div_euclid(WEEK) + 1).max(1) as usize;

    // weekly churn (sparkline) + recency-weighted churn and effective recent-commit
    // count — the latter two are the thrash/excision denominator and its honesty band.
    // One pass. `n_eff = (Σw)²/Σw²` is how much recent data actually backs the rate.
    let mut churn_wk: BTreeMap<i64, f64> = BTreeMap::new();
    let (mut churn_recent, mut sw, mut sw2) = (0.0_f64, 0.0_f64, 0.0_f64);
    for c in commits {
        *churn_wk.entry(week_bucket(c.ts, anchor)).or_default() += c.churn() as f64;
        let w = recency(c.ts, anchor);
        churn_recent += w * c.churn() as f64;
        sw += w;
        sw2 += w * w;
    }
    let churn_recent = churn_recent.max(1.0);
    let n_eff_commits = effective_n(sw, sw2);

    // Survival is fit PER REPO and the weighted rework summed — never a pooled
    // curve (repo frailty differs). Commit clock drives weighting + half-life;
    // wall clock for the days half-life. Half-lives are summarized across repos.
    let mut thr_tot = 0.0;
    let mut exc_tot = 0.0;
    let mut thr_wk: BTreeMap<i64, f64> = BTreeMap::new();
    let mut exc_wk: BTreeMap<i64, f64> = BTreeMap::new();
    // S-weighted thrash split by the intent of the rewriting commit — the input to
    // the thrash qualifier (refactor sweep vs bug-fix churn). Same --me masking.
    let mut thr_by_intent: HashMap<Intent, f64> = HashMap::new();
    let mut survival: Vec<RepoSurvival> = Vec::new();
    for (label, col) in repos {
        // `--me`: keep only deaths I caused (kill_a) for thrash; survival switches
        // to my-authored lines (intro_a). The weighting curve stays the repo's
        // full population — the calibration of how surprising a death at this age is.
        let mask = me.map(|m| col.author_mask(m));
        let ev_c: Vec<f64> = col.deaths.iter().map(|d| d.age_c).collect();
        let (tc, sc) = km_survival(&ev_c, &col.cens_c);
        for d in &col.deaths {
            if let Some(m) = &mask {
                if !(d.kill_a >= 0 && m[d.kill_a as usize]) {
                    continue;
                }
            }
            let w_surv = survival_at(&tc, &sc, d.age_c);
            let wk = week_bucket(d.kill_ts, anchor);
            // Per-week maps feed the sparkline (each week's own value) — survival-weighted
            // only. The totals are *also* recency-weighted, so the headline % reads
            // "lately": recent rework outweighs ancient, and an old repo can still move.
            *thr_wk.entry(wk).or_default() += w_surv * d.rw;
            *exc_wk.entry(wk).or_default() += w_surv * (1.0 - d.rw);
            let w = w_surv * recency(d.kill_ts, anchor);
            thr_tot += w * d.rw;
            exc_tot += w * (1.0 - d.rw);
            *thr_by_intent.entry(d.kill_intent).or_default() += w * d.rw;
        }
        survival.push(survival_row(label, col, mask.as_deref()));
    }
    let thr_pct = thr_tot / churn_recent * 100.0;
    let exc_pct = exc_tot / churn_recent * 100.0;

    let tz = local_offset_secs();
    let mut cards = vec![
        flow_card(&churn_wk),
        batch_card(commits, anchor),
        rate_card("thrash", thr_pct, &thr_wk, &churn_wk, false, n_eff_commits),
        rate_card("excision", exc_pct, &exc_wk, &churn_wk, true, n_eff_commits),
        cadence_card(commits, anchor, tz),
    ];
    qualify_thrash(&mut cards, &thr_by_intent);

    let added: i64 = commits.iter().map(|c| c.added).sum();
    let deleted: i64 = commits.iter().map(|c| c.deleted).sum();
    let net = added - deleted;

    let footer = format!(
        "net {net:+} ({added} added, {deleted} deleted) · \
         run `tv thrash` / `tv hotspots` to drill in"
    );

    Cockpit {
        branch: branch.to_string(),
        // Every figure is on one lens — recency-weighted toward the last few weeks (the
        // exact half-life rides `--explain`); the sparklines show the 8-week trend. The
        // branch label already carries "as of <date>" when anchored.
        window: "lately · recent weeks weighted most".to_string(),
        as_of: as_of.map(str::to_string),
        survival,
        personal: me.is_some(),
        cards,
        footer,
        coverage_weeks,
    }
}

/// Qualify the thrash card by the intent of the *rewriting* work — without moving
/// the % or the tone, only the words. The breakdown always feeds `--explain` (the
/// card's `detail`); when one intent owns the thrash (Pareto vital few ≤ 2) it also
/// rewrites the note — softening a deliberate sweep, sharpening bug-fix churn. A
/// diffuse mix says nothing, so the bare % keeps the high-level signal. This is the
/// one place commit intent reaches a surfaced metric.
fn qualify_thrash(cards: &mut [Card], by_intent: &HashMap<Intent, f64>) {
    let total: f64 = by_intent.values().sum();
    let Some(card) = cards.iter_mut().find(|c| c.key == "thrash") else {
        return;
    };
    if total <= 0.0 {
        return;
    }
    let mut items: Vec<(Intent, f64)> = by_intent.iter().map(|(&i, &v)| (i, v)).collect();
    items.sort_by(|a, b| b.1.total_cmp(&a.1));

    // --explain: the rewrite churn broken down by intent (the top few), composed onto
    // the recency band rate_card already set (don't clobber it — both feed --explain).
    let parts: Vec<String> = items
        .iter()
        .filter(|(_, v)| *v > 0.0)
        .take(4)
        .map(|(i, v)| format!("{} {:.0}%", i.label(), v / total * 100.0))
        .collect();
    let intent = format!("by intent — {}", parts.join(" · "));
    card.detail = Some(match card.detail.take() {
        Some(band) => format!("{intent} · {band}"),
        None => intent,
    });

    // Only "mostly X" when X is genuinely concentrated (the 80% vital few is 1–2
    // intents); otherwise the mix is diffuse and the % stands on its own.
    let shares: Vec<f64> = items.iter().map(|(_, v)| *v).collect();
    if pareto_count(&shares, VITAL_FEW) > 2 {
        return;
    }
    let clause = match items[0].0 {
        Intent::Fix => {
            Some("mostly fixes — code reworked to clear bugs; stabilize this area".to_string())
        }
        Intent::Feature => Some("mostly feature work — reworking new code".to_string()),
        Intent::Revert | Intent::Other => None,
        sweep => Some(format!(
            "mostly {} — a deliberate sweep; sanity-check, then ignore",
            sweep.label()
        )),
    };
    // Re-note only when the thrash is actually flagged — a low/healthy reading needs
    // no qualifier. Band and tone are untouched; only the sentence changes.
    if let (Some(clause), true) = (clause, matches!(card.tone, Tone::Watch | Tone::Alarm)) {
        card.note = Some(format!("{} · {}", card.state, clause));
    }
}

/// Display clamp for the shift residual (σ): caps how far a margin's z-score can read so a
/// tiny-expected outlier can't blow out the scale. Sits just past `render`'s strong-shift
/// cut — the same σ scale, named rather than inline.
const SHIFT_CLAMP_SIGMA: f64 = 4.0;

/// Weekday × hour commit punchcard (local time) — the `cadence` drill-down.
/// Aggregates all commits given; cadence is a rhythm, read across full history.
pub fn cadence_heatmap(commits: &[Commit]) -> Heatmap {
    let tz = local_offset_secs();
    let off = tz.unwrap_or(0);
    let anchor = commits.iter().map(|c| c.ts).max().unwrap_or(0);
    let mut counts = vec![vec![0u32; 24]; 7];
    let mut wsum = vec![vec![0.0_f64; 24]; 7]; // recency-weighted, for the per-margin shift residuals
    let (mut total, mut total_w) = (0u32, 0.0_f64);
    for c in commits {
        let lt = c.ts + off;
        let (d, h) = (weekday_mon0(lt) as usize, hour_of(lt) as usize);
        counts[d][h] += 1;
        total += 1;
        let w = recency(c.ts, anchor);
        wsum[d][h] += w;
        total_w += w;
    }
    // The rhythm grid is the all-time pattern — raw counts, where the medium is strongest.
    let (mut max, mut peak_day, mut peak_hour) = (0u32, 0usize, 0usize);
    for (d, row) in counts.iter().enumerate() {
        for (h, &n) in row.iter().enumerate() {
            if n > max {
                max = n;
                peak_day = d;
                peak_hour = h;
            }
        }
    }
    // The *shift* is the dynamic, but read on the margins rather than per cell: for each
    // whole weekday and each whole hour-of-day, how *surprising* its recent activity is vs
    // that margin's own historical rate — a standardized residual z = (recent − expected) /
    // √expected, expected = the margin's all-time share × the recent volume. A margin (a
    // day, an hour) pools ~24×/~7× the samples of one cell, so the trend is stable: it says
    // which days and which hours are heating/cooling — marked on the grid's own axes, no
    // second grid. The residual is sample-size aware (a busy margin needs a bigger move to
    // register; a stable one reads ~0), and margins below 0.5 expected stay steady. ±4σ.
    let marg_z = |recent: f64, count: u32| -> f64 {
        let expected = if total > 0 {
            count as f64 / total as f64 * total_w
        } else {
            0.0
        };
        if expected >= 0.5 {
            ((recent - expected) / expected.sqrt()).clamp(-SHIFT_CLAMP_SIGMA, SHIFT_CLAMP_SIGMA)
        } else {
            0.0
        }
    };
    let day_shift: Vec<f64> = (0..7)
        .map(|d| marg_z(wsum[d].iter().sum(), counts[d].iter().sum()))
        .collect();
    let hour_shift: Vec<f64> = (0..24)
        .map(|h| {
            let recent: f64 = (0..7).map(|d| wsum[d][h]).sum();
            let count: u32 = (0..7).map(|d| counts[d][h]).sum();
            marg_z(recent, count)
        })
        .collect();
    let s = cadence_shares(commits, off, anchor);
    Heatmap {
        counts,
        max,
        total,
        peak_day,
        peak_hour,
        tz: tz_label(tz),
        day_shift,
        hour_shift,
        weekend_all: s.weekend_all,
        night_all: s.night_all,
        weekend_lately: s.weekend_lately,
        night_lately: s.night_lately,
    }
}

/// One repo's survival row. `mask` None → the whole repo's curve; Some → only the
/// lines the running user introduced (`--me`: "how long the code I write lasts").
fn survival_row(label: &str, col: &Collection, mask: Option<&[bool]>) -> RepoSurvival {
    // No mask → the whole population; with a `--me` mask → only the lines I
    // introduced. One predicate, one pass each, mask-agnostic.
    let keep = |a: i32| mask.is_none() || (a >= 0 && mask.is_some_and(|m| m[a as usize]));
    let (mut ev_c, mut ev_t) = (Vec::new(), Vec::new());
    for d in &col.deaths {
        if keep(d.intro_a) {
            ev_c.push(d.age_c);
            ev_t.push(d.age_t);
        }
    }
    let (mut cens_c, mut cens_t) = (Vec::new(), Vec::new());
    for ((c, t), a) in col.cens_c.iter().zip(&col.cens_t).zip(&col.cens_a) {
        if keep(*a) {
            cens_c.push(*c);
            cens_t.push(*t);
        }
    }
    let (tc, sc) = km_survival(&ev_c, &cens_c);
    let (tt, st) = km_survival(&ev_t, &cens_t);
    let max_age = ev_c.iter().copied().fold(0.0_f64, f64::max);
    let alive = cens_c.len() as f64;
    let total = alive + ev_c.len() as f64;
    RepoSurvival {
        label: label.to_string(),
        curve: sample_survival(&tc, &sc, 24, max_age),
        half_life: half_life_str(half_life(&tc, &sc), half_life(&tt, &st)),
        alive_pct: if total > 0.0 {
            alive / total * 100.0
        } else {
            0.0
        },
    }
}

/// Compact half-life (commit clock / wall clock) for a survival row.
fn half_life_str(c: Option<f64>, d: Option<f64>) -> String {
    match (c, d) {
        (Some(c), Some(d)) => format!("~{c:.0}c / ~{d:.0}d"),
        (Some(c), None) => format!("~{c:.0}c"),
        (None, Some(d)) => format!("~{d:.0}d"),
        (None, None) => "not reached".to_string(),
    }
}

/// The last 8 weeks of a per-week value as a sparkline vector, oldest→newest (so
/// the renderer's bold final bar is the current week). `f(wk)` is the value for
/// week-bucket `wk` (0 = current week), or `None` when that week has no data.
fn weekly_spark(f: impl Fn(i64) -> Option<f64>) -> Vec<f64> {
    let mut v: Vec<(i64, f64)> = (0i64..8).filter_map(|wk| f(wk).map(|x| (wk, x))).collect();
    v.sort_by_key(|p| std::cmp::Reverse(p.0)); // oldest (highest wk) first
    v.into_iter().map(|x| x.1).collect()
}

/// "This week is ramping/rising" cut: above `typical × this` flow reads ramping and batch
/// reads rising. Shared by both cards (the low-side multiplier differs per card and stays
/// local) so the one threshold both lean on can't drift between them.
const TREND_RAMP_MULT: f64 = 1.25;

/// Weekly throughput: this week's churn vs your *recent-typical* weekly churn. "Typical"
/// is a recency-weighted median of the prior weeks (recent weeks count most), so an aging
/// repo's baseline tracks how you work now — not a flat all-history median.
fn flow_card(churn_wk: &BTreeMap<i64, f64>) -> Card {
    let spark_vals = weekly_spark(|wk| churn_wk.get(&wk).copied());

    // Typical = recency-weighted median of the *prior* weeks (exclude the current week, so
    // a busy/quiet week doesn't define its own baseline). Falls back to this week alone.
    let prior: Vec<(f64, f64)> = churn_wk
        .iter()
        .filter(|(&wk, _)| wk >= 1)
        .map(|(&wk, &ch)| (ch, week_decay(wk)))
        .collect();
    let recent = churn_wk.get(&0).copied();
    let typical = if prior.is_empty() {
        recent.unwrap_or(0.0)
    } else {
        weighted_median(&prior)
    };
    let recent = recent.unwrap_or(typical); // partial/empty current week → no false "slowing"

    let ramp_at = typical * TREND_RAMP_MULT;
    let slow_at = typical * 0.70;
    let state = if recent > ramp_at {
        "ramping"
    } else if recent < slow_at {
        "slowing"
    } else {
        "steady"
    }
    .to_string();
    let note = (state == "slowing")
        .then(|| "output dipped — blocked, or just shipping less this week?".to_string());
    let tone = match state.as_str() {
        "slowing" => Tone::Watch,
        "ramping" => Tone::Good,
        _ => Tone::Calm,
    };

    Card {
        key: "flow".to_string(),
        headline: format!("this week ~{recent:.0}/wk · typical ~{typical:.0}"),
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state,
        tone,
        note,
        // --explain: where this week's bands fall, in native units.
        detail: Some(format!(
            "ramps above ~{ramp_at:.0}/wk · slows below ~{slow_at:.0}/wk"
        )),
        available: true,
    }
}

/// Thrash/excision band cut-points (% of recent churn). One source for the cockpit card
/// (`rate_card`) and the drill-down tree-bar color (`render::thr_tone`), so the board and
/// `tv thrash` grade the same percentage identically (the tree advertises "same lens as
/// `tv status`"). Below ELEVATED = healthy throughput; ELEVATED..HIGH = worth a look;
/// HIGH+ = act now. Excision reuses the same cuts with different wording.
pub const THRASH_ELEVATED_PCT: f64 = 8.0;
pub const THRASH_HIGH_PCT: f64 = 15.0;

/// Cadence "heavy" cut-points: a recency-weighted weekend/night commit share above these
/// flags the board (a burnout tripwire, not a verdict). Same class of grade cut as
/// `THRASH_*` — one documented home rather than a function-local literal.
const WEEKEND_HEAVY: f64 = 0.35;
const NIGHT_HEAVY: f64 = 0.25;

/// Thrash / excision as a recency-weighted % of recent churn, with a weekly-rate
/// sparkline. Two halves of the same "code that didn't last", split by `rw`: thrash =
/// rewritten in place (instability), excision = removed outright. Both are S-weighted,
/// which up-weights *young* deaths — so a high reading means recent, normally-durable
/// code is being reworked (thrash) or pulled (excision), not old cruft. The verdict
/// reads off the point estimate so it agrees with the number shown; the Wilson band +
/// `n_eff` ride `--explain`, and the board's `provisional` chip flags thin history.
fn rate_card(
    key: &str,
    pct: f64,
    wk_map: &BTreeMap<i64, f64>,
    churn_wk: &BTreeMap<i64, f64>,
    is_excision: bool,
    n_eff: f64,
) -> Card {
    let spark_vals = weekly_spark(|wk| {
        churn_wk.get(&wk).map(|&ch| {
            let num = wk_map.get(&wk).copied().unwrap_or(0.0);
            if ch > 0.0 {
                num / ch * 100.0
            } else {
                0.0
            }
        })
    });

    // Same band cut-points for both halves (same quantity — S-weighted % of churn); the
    // wording differs because removal is ambiguous where rewrite isn't. Excision: a bit
    // is healthy scope-cutting; a lot of *recent* code pulled is decisive cleanup OR
    // false starts — git can't tell which, so it's a "look", not an alarm.
    let (state, note, tone) = if is_excision {
        if pct >= THRASH_HIGH_PCT {
            (
                "heavy",
                "heavy removal — decisive cleanup, or false starts? worth a look".to_string(),
                Tone::Watch,
            )
        } else if pct >= THRASH_ELEVATED_PCT {
            (
                "pruning",
                "pruning — healthy scope-cutting on recent work".to_string(),
                Tone::Good,
            )
        } else {
            (
                "low",
                "low — little recent work pulled back out".to_string(),
                Tone::Calm,
            )
        }
    } else if pct >= THRASH_HIGH_PCT {
        (
            "high",
            "high — recent code being rewritten; stabilize before adding".to_string(),
            Tone::Alarm,
        )
    } else if pct >= THRASH_ELEVATED_PCT {
        (
            "elevated",
            "elevated — likely a rename/format sweep; sanity-check the area".to_string(),
            Tone::Watch,
        )
    } else {
        (
            "low",
            "low — your speed is real throughput, not thrashing".to_string(),
            Tone::Good,
        )
    };

    // --explain: the Wilson confidence band (proxy — these are churn-weighted, not pure
    // Bernoulli — so n_eff stands in as effective sample size).
    let (lo, hi) = wilson(pct / 100.0, n_eff);
    let detail = Some(format!(
        "~{pct:.1}% [{:.1}–{:.1}%] of recent churn · n_eff {n_eff:.0}",
        lo * 100.0,
        hi * 100.0
    ));
    Card {
        key: key.to_string(),
        headline: format!("~{pct:.1}% of recent churn"),
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state: state.to_string(),
        tone,
        note: Some(note),
        detail,
        available: true,
    }
}

/// Batch size: this week's median lines/commit vs your *recent-typical* (recency-weighted
/// median of prior commits, recent weighted most). Smaller = faster flow.
fn batch_card(commits: &[Commit], anchor: i64) -> Card {
    let mut by_week: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for c in commits {
        by_week
            .entry(week_bucket(c.ts, anchor))
            .or_default()
            .push(c.churn() as f64);
    }
    let spark_vals = weekly_spark(|wk| by_week.get(&wk).map(|v| median(v)));

    // Typical = recency-weighted median of prior-week commits (exclude the current week).
    let prior: Vec<(f64, f64)> = commits
        .iter()
        .filter(|c| week_bucket(c.ts, anchor) >= 1)
        .map(|c| (c.churn() as f64, recency(c.ts, anchor)))
        .collect();
    let recent = by_week.get(&0).map(|v| median(v));
    let typical = if prior.is_empty() {
        recent.unwrap_or(0.0)
    } else {
        weighted_median(&prior)
    };
    let recent = recent.unwrap_or(typical);

    let rise_at = typical * TREND_RAMP_MULT;
    let ease_at = typical * 0.80;
    let state = if recent > rise_at {
        "rising"
    } else if recent < ease_at {
        "easing"
    } else {
        "steady"
    }
    .to_string();

    let note = (state == "rising").then(|| "split smaller — cheapest flow win".to_string());
    let tone = match state.as_str() {
        "rising" => Tone::Watch,
        "easing" => Tone::Good,
        _ => Tone::Calm,
    };
    Card {
        key: "batch".to_string(),
        headline: format!("this week ~{recent:.0}/commit · typical ~{typical:.0}"),
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state,
        tone,
        note,
        detail: Some(format!(
            "rises above ~{rise_at:.0}/commit · eases below ~{ease_at:.0}/commit"
        )),
        available: true,
    }
}

/// Weekend & night commit shares (local time): the all-time rate and the recency-weighted
/// "lately" rate, plus `n_eff` (effective recent commits). One source for both the cadence
/// card and the punchcard, so the card's "lately" and the punchcard's can't drift.
struct CadenceShares {
    weekend_all: f64,
    night_all: f64,
    weekend_lately: f64,
    night_lately: f64,
    n_eff: f64,
}

fn cadence_shares(commits: &[Commit], off: i64, anchor: i64) -> CadenceShares {
    let (mut n, mut we_n, mut ni_n) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut sw, mut sw2, mut sw_we, mut sw_ni) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for c in commits {
        let lt = c.ts + off;
        let (we, ni) = (is_weekend(lt), is_night(lt));
        let w = recency(c.ts, anchor);
        n += 1.0;
        sw += w;
        sw2 += w * w;
        if we {
            we_n += 1.0;
            sw_we += w;
        }
        if ni {
            ni_n += 1.0;
            sw_ni += w;
        }
    }
    let frac = |num: f64, den: f64| if den > 0.0 { num / den } else { 0.0 };
    CadenceShares {
        weekend_all: frac(we_n, n),
        night_all: frac(ni_n, n),
        weekend_lately: frac(sw_we, sw),
        night_lately: frac(sw_ni, sw),
        n_eff: effective_n(sw, sw2),
    }
}

/// Recency-weighted night / weekend share — "how I've been working lately", not a
/// cumulative average an aging repo can't move. The "heavy" call reads off the share
/// itself (point estimate) so it agrees with the number shown; the Wilson band + the
/// `provisional` chip carry confidence. The 8-week night sparkline is the slow view.
fn cadence_card(commits: &[Commit], anchor: i64, tz: Option<i64>) -> Card {
    let off = tz.unwrap_or(0);
    let s = cadence_shares(commits, off, anchor);
    let (weekend, night, n_eff) = (s.weekend_lately, s.night_lately, s.n_eff);

    let mut by_week: BTreeMap<i64, (i64, i64)> = BTreeMap::new(); // (night, total) per week
    for c in commits {
        let e = by_week.entry(week_bucket(c.ts, anchor)).or_insert((0, 0));
        e.1 += 1;
        if is_night(c.ts + off) {
            e.0 += 1;
        }
    }

    let spark_vals = weekly_spark(|w| {
        by_week.get(&w).map(|(n, t)| {
            if *t > 0 {
                *n as f64 / *t as f64 * 100.0
            } else {
                0.0
            }
        })
    });

    // Verdict reads off the recency-weighted share itself, so it always agrees with the
    // number shown. The Wilson band (and the board's `provisional` chip) carry confidence.
    let heavy_weekend = weekend > WEEKEND_HEAVY;
    let heavy_night = night > NIGHT_HEAVY;

    let (state, tone) = if heavy_weekend || heavy_night {
        ("heavy", Tone::Watch)
    } else {
        ("steady", Tone::Calm)
    };
    let note = (heavy_weekend || heavy_night).then(|| {
        let mut parts = Vec::new();
        if heavy_weekend {
            parts.push(format!("{:.0}% weekends", weekend * 100.0));
        }
        if heavy_night {
            parts.push(format!("{:.0}% nights", night * 100.0));
        }
        format!("{} lately — protect recovery time", parts.join(" · "))
    });

    let headline = format!(
        "nights ~{:.0}% · weekends ~{:.0}% ({})",
        night * 100.0,
        weekend * 100.0,
        tz_label(tz)
    );
    // --explain: the Wilson confidence band on each share (n_eff = effective recent commits).
    let (wk_lo, wk_hi) = wilson(weekend, n_eff);
    let (nt_lo, nt_hi) = wilson(night, n_eff);
    let detail = Some(format!(
        "weekends ~{:.0}% [{:.0}–{:.0}%] · nights ~{:.0}% [{:.0}–{:.0}%] · n_eff {:.0}",
        weekend * 100.0,
        wk_lo * 100.0,
        wk_hi * 100.0,
        night * 100.0,
        nt_lo * 100.0,
        nt_hi * 100.0,
        n_eff
    ));

    Card {
        key: "cadence".to_string(),
        headline,
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state: state.to_string(),
        tone,
        note,
        detail,
        available: true,
    }
}

/// A folder in the thrash tree. Each node accumulates the S-weighted thrash /
/// excision / churn of everything beneath it.
#[derive(Default)]
pub struct TreeNode {
    pub thrash: f64,
    pub thrash_recent: f64, // subset of thrash in the last 7 days, for trajectory
    pub excision: f64,
    pub churn: f64,
    pub children: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    /// Thrash as a % of this folder's churn — the node's defining ratio (0 when no churn).
    /// One definition so the terminal and report read the same number at every node.
    pub fn thrash_pct(&self) -> f64 {
        if self.churn > 0.0 {
            self.thrash / self.churn * 100.0
        } else {
            0.0
        }
    }
}

/// Directory segments of a path; root-level files bucket under "(root)".
fn dir_segments(path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop(); // drop the filename — we group by folder
    if parts.is_empty() {
        vec!["(root)".to_string()]
    } else {
        parts.into_iter().map(str::to_string).collect()
    }
}

fn insert(
    node: &mut TreeNode,
    segs: &[String],
    thrash: f64,
    recent: f64,
    excision: f64,
    churn: f64,
) {
    node.thrash += thrash;
    node.thrash_recent += recent;
    node.excision += excision;
    node.churn += churn;
    if let Some((head, rest)) = segs.split_first() {
        insert(
            node.children.entry(head.clone()).or_default(),
            rest,
            thrash,
            recent,
            excision,
            churn,
        );
    }
}

/// Prepend the repo label as the top folder segment when aggregating several.
fn repo_segments(label: &str, path: &str, multi: bool) -> Vec<String> {
    let segs = dir_segments(path);
    if multi {
        std::iter::once(label.to_string()).chain(segs).collect()
    } else {
        segs
    }
}

/// Build the folder tree of thrash, on the *same lens as the cockpit* so the root
/// reconciles with the `tv status` thrash card: each death weighted by survival ×
/// recency (`all_time` drops the recency factor → flat lifetime totals), over all
/// history. The churn denominator arrives pre-weighted on the same lens. Each repo's
/// rework uses its OWN survival curve, and (when several) sits under a repo node.
pub fn thrash_tree(
    repos: &[(String, Collection)],
    churn: &[RepoChurn],
    anchor: i64,
    all_time: bool,
    recent_cut: Option<i64>,
    multi: bool,
    me: Option<&Me>,
) -> TreeNode {
    let mut root = TreeNode::default();
    for (label, col) in repos {
        let mask = me.map(|m| col.author_mask(m));
        let ev_c: Vec<f64> = col.deaths.iter().map(|d| d.age_c).collect();
        let (tc, sc) = km_survival(&ev_c, &col.cens_c); // per-repo S (full calibration)
        for d in &col.deaths {
            if let Some(m) = &mask {
                if !(d.kill_a >= 0 && m[d.kill_a as usize]) {
                    continue; // --me: only rework I did
                }
            }
            let r = if all_time {
                1.0
            } else {
                recency(d.kill_ts, anchor)
            };
            let w = survival_at(&tc, &sc, d.age_c) * r;
            let t = w * d.rw;
            let t_recent = if recent_cut.is_some_and(|rc| d.kill_ts >= rc) {
                t
            } else {
                0.0
            };
            insert(
                &mut root,
                &repo_segments(label, &d.path, multi),
                t,
                t_recent,
                w * (1.0 - d.rw),
                0.0,
            );
        }
    }
    for (label, cmap) in churn {
        for (path, &ch) in cmap {
            insert(
                &mut root,
                &repo_segments(label, path, multi),
                0.0,
                0.0,
                0.0,
                ch,
            );
        }
    }
    root
}

/// One repo's hotspot inputs: (label, change-frequency `(count, recency_sum)` by path,
/// complexity by path).
pub type RepoFiles = (String, HashMap<String, (i64, f64)>, HashMap<String, i64>);

/// One repo's thrash denominator: (label, recency-weighted churn by path).
pub type RepoChurn = (String, HashMap<String, f64>);

pub struct Hotspot {
    pub repo: String, // owning repo label (shown only when aggregating several)
    pub file: String,
    pub freq: i64,       // raw commits touching the file (shown for intuition)
    pub complexity: i64, // nesting-depth proxy
    pub recency: f64,    // mean recency of those commits (0 = ancient, 1 = today)
    pub score: f64,      // freq × complexity × recency — recently-hot AND deep ranks top
}

/// The "vital few" share: how much of a total a self-scaling cut should keep. The
/// Pareto 80/20 — show the smallest set carrying 80% of the heat, so the count
/// follows the distribution (peaked → few, diffuse → more) instead of a magic N.
pub const VITAL_FEW: f64 = 0.8;

/// Smallest prefix of a descending-sorted score list whose sum reaches `share` of
/// the total — the cut that keeps the vital few. `0` for an empty/zero list;
/// otherwise ≥1. The single self-calibrating limiter for hotspots and the tree.
pub fn pareto_count(desc: &[f64], share: f64) -> usize {
    let total: f64 = desc.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let cut = total * share;
    let mut acc = 0.0;
    for (i, &v) in desc.iter().enumerate() {
        acc += v;
        if acc >= cut {
            return i + 1;
        }
    }
    desc.len()
}

/// Files ranked by change-frequency × complexity × recency — edited often AND deeply
/// nested AND recently is the highest-ROI refactor target (the recency factor, on the
/// same lens as the rest of the tool, demotes files that were hot long ago). Ranked
/// across all repos (each row keeps its repo so same-named files stay distinct), fully
/// sorted; the view applies the [`pareto_count`] cut so there's no arbitrary top-N here.
pub fn hotspots(per_repo: &[RepoFiles]) -> Vec<Hotspot> {
    let mut rows: Vec<Hotspot> = Vec::new();
    for (label, freq, complexity) in per_repo {
        for (file, &cx) in complexity {
            let Some(&(count, wsum)) = freq.get(file) else {
                continue;
            };
            if count > 0 && cx > 0 {
                let recency = wsum / count as f64; // mean recency of this file's edits
                rows.push(Hotspot {
                    repo: label.clone(),
                    file: file.clone(),
                    freq: count,
                    complexity: cx,
                    recency,
                    score: wsum * cx as f64, // = count × cx × recency
                });
            }
        }
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_is_one_at_anchor_and_halves_at_halflife() {
        let anchor = 1_000_000_000;
        assert!((recency(anchor, anchor) - 1.0).abs() < 1e-9);
        let hl = (RECENCY_HALFLIFE_DAYS * DAY as f64) as i64;
        assert!((recency(anchor - hl, anchor) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn recency_decreases_with_age_and_clamps_future() {
        let a = 1_000_000_000;
        assert!(recency(a - 100_000, a) < recency(a - 1_000, a));
        assert!((recency(a + 5_000, a) - 1.0).abs() < 1e-9); // future ts clamps to anchor
    }
}
