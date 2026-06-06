//! Build the one-screen cockpit from commit data + the survival collection.

use std::collections::{BTreeMap, HashMap};

use crate::git::Collection;
use crate::model::{Card, Cockpit, Commit, Heatmap, Me, RepoSurvival, Tone};
use crate::spark::{median, percentile_rank, sparkline};
use crate::survival::{half_life, km_survival, sample_survival, survival_at};
use crate::verdict::{self, Signals};

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

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
) -> Cockpit {
    let anchor = commits.iter().map(|c| c.ts).max().unwrap_or(0);
    let min_ts = commits.iter().map(|c| c.ts).min().unwrap_or(anchor);
    let coverage_weeks = ((anchor - min_ts).div_euclid(WEEK) + 1).max(1) as usize;

    // weekly churn — shared by flow and the thrash/excision rates
    let mut churn_wk: BTreeMap<i64, f64> = BTreeMap::new();
    for c in commits {
        *churn_wk.entry(week_bucket(c.ts, anchor)).or_default() += c.churn() as f64;
    }
    let total_churn: f64 = commits
        .iter()
        .map(|c| c.churn() as f64)
        .sum::<f64>()
        .max(1.0);

    // Survival is fit PER REPO and the weighted rework summed — never a pooled
    // curve (repo frailty differs). Commit clock drives weighting + half-life;
    // wall clock for the days half-life. Half-lives are summarized across repos.
    let mut thr_tot = 0.0;
    let mut exc_tot = 0.0;
    let mut thr_wk: BTreeMap<i64, f64> = BTreeMap::new();
    let mut exc_wk: BTreeMap<i64, f64> = BTreeMap::new();
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
            let w = survival_at(&tc, &sc, d.age_c);
            let wk = week_bucket(d.kill_ts, anchor);
            thr_tot += w * d.rw;
            exc_tot += w * (1.0 - d.rw);
            *thr_wk.entry(wk).or_default() += w * d.rw;
            *exc_wk.entry(wk).or_default() += w * (1.0 - d.rw);
        }
        survival.push(survival_row(label, col, mask.as_deref()));
    }
    let thr_pct = thr_tot / total_churn * 100.0;
    let exc_pct = exc_tot / total_churn * 100.0;

    let (batch, batch_state, batch_from, batch_to) = batch_card(commits, anchor);
    let tz = local_offset_secs();
    let (cadence, nights_recent, nights_base, weekend_pct) = cadence_card(commits, anchor, tz);

    let cards = vec![
        flow_card(&churn_wk),
        batch,
        rate_card("thrash", thr_pct, &thr_wk, &churn_wk, false),
        rate_card("excision", exc_pct, &exc_wk, &churn_wk, true),
        cadence,
    ];

    let added: i64 = commits.iter().map(|c| c.added).sum();
    let deleted: i64 = commits.iter().map(|c| c.deleted).sum();
    let net = added - deleted;

    let sig = Signals {
        coverage_weeks,
        batch_state,
        batch_from,
        batch_to,
        nights_recent,
        nights_base,
        weekend_pct,
        thrash_pct: thr_pct,
        net,
    };
    let verdict = verdict::compose(&sig);

    let footer = format!(
        "net {net:+} ({added} added, {deleted} deleted) · \
         run `tv thrash` / `tv hotspots` to drill in"
    );

    Cockpit {
        branch: branch.to_string(),
        window: "last 7d vs trailing 8wk".to_string(),
        verdict,
        survival,
        personal: me.is_some(),
        cards,
        footer,
        coverage_weeks,
    }
}

/// Weekday × hour commit punchcard (local time) — the `cadence` drill-down.
/// Aggregates all commits given; cadence is a rhythm, read across full history.
pub fn cadence_heatmap(commits: &[Commit]) -> Heatmap {
    let tz = local_offset_secs();
    let off = tz.unwrap_or(0);
    let mut counts = vec![vec![0u32; 24]; 7];
    let (mut total, mut weekend, mut night) = (0u32, 0u32, 0u32);
    for c in commits {
        let lt = c.ts + off;
        counts[weekday_mon0(lt) as usize][hour_of(lt) as usize] += 1;
        total += 1;
        if is_weekend(lt) {
            weekend += 1;
        }
        if is_night(lt) {
            night += 1;
        }
    }
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
    let denom = total.max(1) as f64;
    Heatmap {
        counts,
        max,
        total,
        peak_day,
        peak_hour,
        tz: tz_label(tz),
        weekend_pct: weekend as f64 / denom * 100.0,
        night_pct: night as f64 / denom * 100.0,
    }
}

/// One repo's survival row. `mask` None → the whole repo's curve; Some → only the
/// lines the running user introduced (`--me`: "how long the code I write lasts").
fn survival_row(label: &str, col: &Collection, mask: Option<&[bool]>) -> RepoSurvival {
    let mine = |a: i32| a >= 0 && mask.is_some_and(|m| m[a as usize]);
    let (ev_c, ev_t, cens_c, cens_t): (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) = if mask.is_none() {
        (
            col.deaths.iter().map(|d| d.age_c).collect(),
            col.deaths.iter().map(|d| d.age_t).collect(),
            col.cens_c.clone(),
            col.cens_t.clone(),
        )
    } else {
        let ev_c = col
            .deaths
            .iter()
            .filter(|d| mine(d.intro_a))
            .map(|d| d.age_c)
            .collect();
        let ev_t = col
            .deaths
            .iter()
            .filter(|d| mine(d.intro_a))
            .map(|d| d.age_t)
            .collect();
        let mut cc = Vec::new();
        let mut ct = Vec::new();
        for ((c, t), a) in col.cens_c.iter().zip(&col.cens_t).zip(&col.cens_a) {
            if mine(*a) {
                cc.push(*c);
                ct.push(*t);
            }
        }
        (ev_c, ev_t, cc, ct)
    };
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

/// Weekly throughput (total churn/week) — a steadiness pulse, self-relative.
fn flow_card(churn_wk: &BTreeMap<i64, f64>) -> Card {
    let mut v: Vec<(i64, f64)> = (0i64..8)
        .filter_map(|wk| churn_wk.get(&wk).map(|&c| (wk, c)))
        .collect();
    v.sort_by_key(|p| std::cmp::Reverse(p.0));
    let spark_vals: Vec<f64> = v.iter().map(|x| x.1).collect();

    let weekly: Vec<f64> = churn_wk.values().copied().collect();
    let overall = median(&weekly);
    let recent = churn_wk.get(&0).copied().unwrap_or(overall);
    let state = if recent > overall * 1.25 {
        "ramping"
    } else if recent < overall * 0.7 {
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
        headline: format!("~{overall:.0} lines/wk"),
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state,
        tone,
        note,
        available: true,
    }
}

/// Thrash / excision as a % of churn, with a weekly-rate sparkline.
fn rate_card(
    key: &str,
    pct: f64,
    wk_map: &BTreeMap<i64, f64>,
    churn_wk: &BTreeMap<i64, f64>,
    healthy: bool,
) -> Card {
    let mut v: Vec<(i64, f64)> = (0i64..8)
        .filter_map(|wk| {
            churn_wk.get(&wk).map(|&ch| {
                let num = wk_map.get(&wk).copied().unwrap_or(0.0);
                (wk, if ch > 0.0 { num / ch * 100.0 } else { 0.0 })
            })
        })
        .collect();
    v.sort_by_key(|p| std::cmp::Reverse(p.0));
    let spark_vals: Vec<f64> = v.iter().map(|x| x.1).collect();

    let (state, note) = if healthy {
        ("healthy", "deliberate scope-cutting (healthy)".to_string())
    } else if pct < 8.0 {
        (
            "low",
            "low — your speed is real throughput, not thrashing".to_string(),
        )
    } else if pct < 15.0 {
        (
            "elevated",
            "elevated — likely a rename/format sweep; sanity-check the area".to_string(),
        )
    } else {
        (
            "high",
            "high — recent code being rewritten; stabilize before adding".to_string(),
        )
    };

    let tone = if healthy || pct < 8.0 {
        Tone::Good
    } else if pct < 15.0 {
        Tone::Watch
    } else {
        Tone::Alarm
    };
    Card {
        key: key.to_string(),
        headline: format!("{pct:.1}% of churn"),
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state: state.to_string(),
        tone,
        note: Some(note),
        available: true,
    }
}

/// Returns (card, state, overall_median, recent_median).
fn batch_card(commits: &[Commit], anchor: i64) -> (Card, String, f64, f64) {
    let mut by_week: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    let mut all: Vec<f64> = Vec::with_capacity(commits.len());
    for c in commits {
        let churn = c.churn() as f64;
        all.push(churn);
        by_week
            .entry(week_bucket(c.ts, anchor))
            .or_default()
            .push(churn);
    }

    let overall = median(&all);
    let recent = by_week.get(&0).map(|v| median(v)).unwrap_or(overall);

    let mut wk_med: Vec<(i64, f64)> = (0i64..8)
        .filter_map(|wk| by_week.get(&wk).map(|v| (wk, median(v))))
        .collect();
    wk_med.sort_by_key(|p| std::cmp::Reverse(p.0)); // oldest (highest wk) first
    let spark_vals: Vec<f64> = wk_med.iter().map(|x| x.1).collect();

    let week_medians: Vec<f64> = by_week.values().map(|v| median(v)).collect();
    let pct = percentile_rank(recent, &week_medians);

    let state = if recent > overall * 1.25 {
        "rising"
    } else if recent < overall * 0.8 {
        "easing"
    } else {
        "steady"
    }
    .to_string();

    let headline = format!("median {overall:.0}→{recent:.0} (p{pct:.0} for you)");
    let note = (state == "rising").then(|| "split smaller — cheapest flow win".to_string());

    let tone = match state.as_str() {
        "rising" => Tone::Watch,
        "easing" => Tone::Good,
        _ => Tone::Calm,
    };
    let card = Card {
        key: "batch".to_string(),
        headline,
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state: state.clone(),
        tone,
        note,
        available: true,
    };
    (card, state, overall, recent)
}

/// Returns (card, recent_night%, baseline_night%, weekend%).
fn cadence_card(commits: &[Commit], anchor: i64, tz: Option<i64>) -> (Card, f64, f64, f64) {
    let off = tz.unwrap_or(0);
    let mut by_week: BTreeMap<i64, (i64, i64)> = BTreeMap::new(); // (night, total)
    let mut tot_night = 0_i64;
    let mut tot_weekend = 0_i64;
    for c in commits {
        let e = by_week.entry(week_bucket(c.ts, anchor)).or_insert((0, 0));
        e.1 += 1;
        if is_night(c.ts + off) {
            e.0 += 1;
            tot_night += 1;
        }
        if is_weekend(c.ts + off) {
            tot_weekend += 1;
        }
    }
    let total = commits.len().max(1) as f64;
    let baseline_night = tot_night as f64 / total * 100.0;
    let weekend_pct = tot_weekend as f64 / total * 100.0;

    let recent: Vec<&Commit> = commits.iter().filter(|c| c.ts >= anchor - WEEK).collect();
    let recent_night = if recent.is_empty() {
        baseline_night
    } else {
        recent.iter().filter(|c| is_night(c.ts + off)).count() as f64 / recent.len() as f64 * 100.0
    };

    let mut wk: Vec<(i64, f64)> = (0i64..8)
        .filter_map(|w| {
            by_week.get(&w).map(|(n, t)| {
                let share = if *t > 0 {
                    *n as f64 / *t as f64 * 100.0
                } else {
                    0.0
                };
                (w, share)
            })
        })
        .collect();
    wk.sort_by_key(|p| std::cmp::Reverse(p.0));
    let spark_vals: Vec<f64> = wk.iter().map(|x| x.1).collect();

    // Flag both drift (recent night share climbing) and sustained high level —
    // for a solo builder the *level* of night/weekend work is the real signal,
    // and a chronically-high-but-stable level won't show up as drift.
    let rising = recent_night > baseline_night + 7.0;
    let heavy_weekend = weekend_pct > 35.0;
    let heavy_night = baseline_night > 25.0;

    let state = if rising {
        "nights ↑"
    } else if heavy_weekend || heavy_night {
        "heavy"
    } else {
        "steady"
    }
    .to_string();
    let tone = if rising || heavy_weekend || heavy_night {
        Tone::Watch
    } else {
        Tone::Calm
    };

    let note = if rising {
        Some(format!(
            "night share climbing {baseline_night:.0}→{recent_night:.0}% — protect rest"
        ))
    } else if heavy_weekend || heavy_night {
        let mut parts = Vec::new();
        if heavy_weekend {
            parts.push(format!("{weekend_pct:.0}% of commits on weekends"));
        }
        if heavy_night {
            parts.push(format!("{baseline_night:.0}% at night"));
        }
        Some(format!("{} — protect recovery time", parts.join(" · ")))
    } else {
        None
    };

    // headline shows the sustained level (all-time), not the 7-day window
    let headline = format!(
        "nights {baseline_night:.0}% · weekends {weekend_pct:.0}% ({})",
        tz_label(tz)
    );

    let card = Card {
        key: "cadence".to_string(),
        headline,
        spark: sparkline(&spark_vals),
        spark_values: spark_vals,
        state,
        tone,
        note,
        available: true,
    };
    (card, recent_night, baseline_night, weekend_pct)
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
        let mut v = Vec::with_capacity(segs.len() + 1);
        v.push(label.to_string());
        v.extend(segs);
        v
    } else {
        segs
    }
}

/// Build the folder tree of S-weighted thrash. The root holds the totals; every
/// node holds the thrash (and churn, for the %) of its whole subtree. Each repo's
/// rework is weighted by its OWN survival curve, and (when several) sits under a
/// top-level node named for the repo.
pub fn thrash_tree(
    repos: &[(String, Collection)],
    churn: &[RepoChurn],
    since: Option<i64>,
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
            if since.is_some_and(|cut| d.kill_ts < cut) {
                continue; // only aggregate rewrites inside the window
            }
            if let Some(m) = &mask {
                if !(d.kill_a >= 0 && m[d.kill_a as usize]) {
                    continue; // --me: only rework I did
                }
            }
            let w = survival_at(&tc, &sc, d.age_c);
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
                ch as f64,
            );
        }
    }
    root
}

/// One repo's hotspot inputs: (label, change-frequency by path, complexity by path).
pub type RepoFiles = (String, HashMap<String, i64>, HashMap<String, i64>);

/// One repo's thrash denominator: (label, churn by path).
pub type RepoChurn = (String, HashMap<String, i64>);

pub struct Hotspot {
    pub repo: String, // owning repo label (shown only when aggregating several)
    pub file: String,
    pub freq: i64, // commits touching the file in the window
    pub complexity: i64,
    pub score: f64,
}

/// Files ranked by change-frequency × complexity — files edited often AND deeply
/// nested are the highest-ROI refactor targets. Ranked across all repos, but each
/// row keeps its repo so same-named files in different repos stay distinct.
pub fn hotspots(per_repo: &[RepoFiles], top: usize) -> Vec<Hotspot> {
    let mut rows: Vec<Hotspot> = Vec::new();
    for (label, freq, complexity) in per_repo {
        for (file, &cx) in complexity {
            let Some(&f) = freq.get(file) else { continue };
            if f > 0 && cx > 0 {
                rows.push(Hotspot {
                    repo: label.clone(),
                    file: file.clone(),
                    freq: f,
                    complexity: cx,
                    score: f as f64 * cx as f64,
                });
            }
        }
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(top);
    rows
}
