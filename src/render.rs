//! Output skins. Terminal cockpit is the default (daily glance); the HTML
//! report (the `report` command / `--report`) is the manager/retro skin —
//! cockpit + thrash + hotspots on one page. Both lead with the verdict.

use std::fs;

use crate::metrics::{Hotspot, TreeNode};
use crate::model::{Card, Cockpit, Heatmap, RepoSurvival, Tone};
use crate::spark::sparkline;
use crate::style::Palette;

const WIDTH: usize = 60;
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn rule() -> String {
    "─".repeat(WIDTH)
}

/// Naive word-wrap (byte-width; fine for the mostly-ASCII verdict/footer).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.len() + 1 + word.len() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

pub fn print_cockpit(c: &Cockpit, p: &Palette) {
    let dim_rule = p.dim(&rule());
    println!(
        "{} {}",
        p.bold("terminal velocity"),
        p.dim(&format!("· {} · {}", c.branch, c.window))
    );
    println!("{dim_rule}");

    // verdict is colored by the worst tone on the board — the cockpit's mood
    let worst = c
        .cards
        .iter()
        .map(|x| x.tone)
        .max_by_key(|t| t.rank())
        .unwrap_or(Tone::Calm);
    for line in wrap(&c.verdict, WIDTH) {
        println!("{}", p.mood(worst, &line));
    }
    println!("{dim_rule}");

    if !c.survival.is_empty() {
        print_survival(&c.survival, c.personal, p);
        println!("{dim_rule}");
    }

    for card in &c.cards {
        print_card(card, p);
    }
    println!("{dim_rule}");
    for line in wrap(&c.footer, WIDTH) {
        println!("{}", p.dim(&line));
    }
}

fn print_card(card: &Card, p: &Palette) {
    let key = p.bold(&format!("{:<9}", card.key));
    let spark = decorate_spark(p, card.tone, &card.spark);
    let state = p.tone(card.tone, &card.state);
    if card.available {
        println!("  {key} {spark} {state} · {}", card.headline);
    } else {
        println!("  {key} {spark} {state}");
    }
    if let Some(note) = &card.note {
        println!("{}", p.dim(&format!("             └ {note}")));
    }
}

/// The survival curve(s) — S(age) — that weight every thrash/excision, surfaced
/// right under the verdict. One repo: curve + half-life + alive%, with a one-line
/// gloss. Several: one compact row per repo (S is fit per repo).
fn print_survival(survivals: &[RepoSurvival], personal: bool, p: &Palette) {
    // Downsample to a tidy 16-char sparkline (the stored curve is finer for HTML).
    let spark = |c: &[f64]| -> String {
        if c.len() < 2 {
            return "—".to_string();
        }
        if c.len() <= 16 {
            return sparkline(c);
        }
        let step = c.len() as f64 / 16.0;
        let ds: Vec<f64> = (0..16)
            .map(|i| c[((i as f64 * step) as usize).min(c.len() - 1)])
            .collect();
        sparkline(&ds)
    };
    let title = if personal {
        "my code survival"
    } else {
        "code survival"
    };

    if survivals.len() == 1 {
        let s = &survivals[0];
        println!(
            "{}  {}  half-life {} · {}",
            p.bold(title),
            spark(&s.curve),
            p.bold(&s.half_life),
            p.dim(&format!("{:.0}% of lines still alive", s.alive_pct)),
        );
        let gloss = if personal {
            "how long the lines you write survive (S(age) over your own code)."
        } else {
            "S(age) = a deleted line's odds of having lived this long; \
             thrash and excision weight every death by it."
        };
        for line in wrap(gloss, WIDTH - 2) {
            println!("{}", p.dim(&format!("  {line}")));
        }
    } else {
        let tag = if personal {
            "· how long the lines you write survive · per repo"
        } else {
            "· S(age) weights every thrash & excision · fit per repo"
        };
        println!("{} {}", p.bold(title), p.dim(tag));
        for s in survivals {
            println!(
                "  {} {}  {} · {}",
                p.bold(&trunc(&s.label, 16)),
                spark(&s.curve),
                p.dim(&s.half_life),
                p.dim(&format!("{:.0}% alive", s.alive_pct)),
            );
        }
    }
}

/// Sparkline runs oldest→newest. The final bar is *this week* — the value the
/// state and headline are judging — so it gets emphasis (bold) while the history
/// recedes (dim). Padded to a fixed width with plain trailing spaces.
fn decorate_spark(p: &Palette, tone: Tone, spark: &str) -> String {
    const W: usize = 8;
    let chars: Vec<char> = spark.chars().collect();
    if chars.is_empty() {
        return " ".repeat(W);
    }
    let n = chars.len();
    let head: String = chars[..n - 1].iter().collect();
    let now = chars[n - 1].to_string();
    let body = format!(
        "{}{}",
        p.dim(&p.tone(tone, &head)),
        p.bold(&p.tone(tone, &now)),
    );
    format!("{body}{}", " ".repeat(W.saturating_sub(n)))
}

/// Filled bar (tone-colored) + dim remainder, fixed width.
fn hbar(p: &Palette, frac: f64, width: usize, tone: Tone) -> String {
    let filled = (frac.clamp(0.0, 1.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!(
        "{}{}",
        p.tone(tone, &"█".repeat(filled)),
        p.dim(&"░".repeat(width - filled))
    )
}

/// Truncate-or-pad to an exact visible width (keeps the informative tail).
fn trunc(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len > n {
        let tail: String = s.chars().skip(len - (n - 1)).collect();
        return format!("…{tail}");
    }
    let mut out = s.to_string();
    out.push_str(&" ".repeat(n - len));
    out
}

fn thr_tone(pct: f64) -> Tone {
    if pct < 8.0 {
        Tone::Good
    } else if pct < 15.0 {
        Tone::Watch
    } else {
        Tone::Alarm
    }
}

pub fn print_thrash(branch: &str, root: &TreeNode, recent: bool, p: &Palette) {
    let window = if recent { "last 8wk" } else { "all-time" };
    println!(
        "{} {}",
        p.bold("tv thrash"),
        p.dim(&format!("· {branch} · {window}"))
    );
    println!("{}", p.dim(&rule()));
    println!(
        "{}",
        p.dim("in-place rewrite: recently-written code rewritten again, weighted")
    );
    println!(
        "{}",
        p.dim("by how recent. by folder. % = thrash as a share of that folder's churn.")
    );
    if recent {
        println!(
            "{}",
            p.dim("↑ heating / ↓ cooling = last 7d vs the 8-week pace.")
        );
    }
    println!();
    if root.thrash <= 0.0 || root.children.is_empty() {
        println!("  (no rework recorded)");
        return;
    }
    // Prune folders below 2.5% of total thrash — depth follows naturally.
    let min = (root.thrash * 0.025).max(1.0);
    let scale = root
        .children
        .values()
        .map(|c| c.thrash)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let mut shown = 0usize;
    print_branch(p, root, scale, min, "", recent, &mut shown);

    println!("{}", p.dim(&rule()));
    println!(
        "{}",
        p.dim(&format!(
            "total thrash {:.0} · excision {:.0} (deliberate removals, not rework)",
            root.thrash, root.excision
        ))
    );
}

/// Trend arrow for a folder: last-7d thrash vs its proportional 8-week share
/// (~1/8). Heating up ↑, cooling ↓, steady →. A space when not in the windowed view.
fn traj_arrow(p: &Palette, recent: bool, node: &TreeNode) -> String {
    if !recent {
        return " ".to_string();
    }
    let r = if node.thrash > 0.0 {
        node.thrash_recent / node.thrash
    } else {
        0.0
    };
    if r > 0.20 {
        p.yellow("↑")
    } else if r < 0.06 {
        p.green("↓")
    } else {
        p.dim("→")
    }
}

/// Print a folder's children as an indented tree, biggest first, pruning < `min`.
fn print_branch(
    p: &Palette,
    node: &TreeNode,
    scale: f64,
    min: f64,
    prefix: &str,
    recent: bool,
    shown: &mut usize,
) {
    let mut kids: Vec<(&String, &TreeNode)> = node
        .children
        .iter()
        .filter(|(_, c)| c.thrash >= min)
        .collect();
    kids.sort_by(|a, b| {
        b.1.thrash
            .partial_cmp(&a.1.thrash)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let n = kids.len();
    for (i, (name, child)) in kids.into_iter().enumerate() {
        if *shown >= 40 {
            return; // hard safety cap; the prune normally bounds it well below
        }
        *shown += 1;
        let last = i == n - 1;
        let connector = if last { "└─ " } else { "├─ " };
        let pct = if child.churn > 0.0 {
            child.thrash / child.churn * 100.0
        } else {
            0.0
        };
        let label = trunc(&format!("{prefix}{connector}{name}"), 30);
        println!(
            "  {label} {} {}  {}  {}",
            hbar(p, child.thrash / scale, 8, thr_tone(pct)),
            traj_arrow(p, recent, child),
            p.bold(&format!("{:>5.0}", child.thrash)),
            p.dim(&format!("· {pct:.0}%")),
        );
        let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
        print_branch(p, child, scale, min, &child_prefix, recent, shown);
    }
}

pub fn print_hotspots(rows: &[Hotspot], recent: bool, multi: bool, p: &Palette) {
    let window = if recent { "last 8wk" } else { "all-time" };
    println!(
        "{} {}",
        p.bold("tv hotspots"),
        p.dim(&format!("· {window} · revisions × complexity"))
    );
    println!("{}", p.dim(&rule()));
    println!(
        "{}",
        p.dim("files changed often AND deeply nested — refactoring these pays off most.")
    );
    println!(
        "{}",
        p.dim("Nx = commits that touched the file · cx = indentation complexity (nesting).")
    );
    println!();
    if rows.is_empty() {
        println!("  (no files)");
        return;
    }
    let max = rows
        .iter()
        .map(|r| r.score)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    for r in rows {
        // sqrt scaling so the list past the top file stays readable; the numbers
        // carry the real magnitude.
        let stats = if multi {
            format!("{}× changed · cx {} · {}", r.freq, r.complexity, r.repo)
        } else {
            format!("{}× changed · cx {}", r.freq, r.complexity)
        };
        println!(
            "  {}  {} {}",
            hbar(p, (r.score / max).sqrt(), 10, Tone::Watch),
            trunc(&r.file, 38),
            p.dim(&stats),
        );
    }
}

/// One punchcard cell: shaded block scaled to the busiest cell (GitHub-green).
fn heat_cell(p: &Palette, count: u32, max: u32) -> String {
    if count == 0 {
        return "  ".to_string();
    }
    let lvl = if max == 0 {
        1
    } else {
        ((count as f64 / max as f64) * 4.0).ceil().clamp(1.0, 4.0) as u8
    };
    match lvl {
        1 => p.dim("░░"),
        2 => p.green("▒▒"),
        3 => p.green("▓▓"),
        _ => p.bold(&p.green("██")),
    }
}

/// The cadence drill-down: a weekday × hour commit punchcard (local time).
pub fn print_heatmap(h: &Heatmap, scope: &str, p: &Palette) {
    println!(
        "{} {}",
        p.bold("tv cadence"),
        p.dim(&format!(
            "· {scope} · when commits land · {} · all history",
            h.tz
        ))
    );
    println!("{}", p.dim(&rule()));
    if h.total == 0 {
        println!("  (no commits)");
        return;
    }
    // hour axis: 5-char day gutter, then a tick every 3 hours (2 cols each).
    let mut axis = String::from("     ");
    for hh in (0..24).step_by(3) {
        axis.push_str(&format!("{hh:<6}"));
    }
    println!("{}", p.dim(&axis));

    for (d, row) in h.counts.iter().enumerate() {
        let cells: String = (0..24).map(|hh| heat_cell(p, row[hh], h.max)).collect();
        let day = p.bold(&format!("{:<3}", DAYS[d]));
        println!(" {day} {cells}");
    }

    println!("{}", p.dim(&rule()));
    println!(
        "  {}  ·  {}",
        p.bold(&format!(
            "peak {} {:02}:00 ({} commits)",
            DAYS[h.peak_day], h.peak_hour, h.max
        )),
        p.dim(&format!(
            "{:.0}% weekend · {:.0}% night",
            h.weekend_pct, h.night_pct
        )),
    );
    println!(
        "{}",
        p.dim(&format!(
            "  less {} {} {} {} more · night = 20:00–06:00",
            p.dim("░░"),
            p.green("▒▒"),
            p.green("▓▓"),
            p.bold(&p.green("██")),
        ))
    );
}

/// The heuristic decision tree, drawn in the terminal. Mirror of the logic in
/// intent.rs / metrics.rs / verdict.rs — keep in sync when thresholds change.
pub fn print_explain(p: &Palette) {
    let raw = "\
terminal velocity · decision tree
────────────────────────────────────────────────────────────
how every word in the cockpit is decided.
  ● self-calibrated to your repo (no magic number)
  ○ tunable constant (the only hand-set knobs)
  ▸ sparklines run 8 weeks old→new — the bold last bar is this week

INTENT · per commit, first match wins
├─ subject has \"revert\" ···················· revert
├─ subject has fix/bug/resolve/correct ······ fix
├─ ○ deleted > 2×added AND >15 lines ········ refactor
│    └ …or retire/drop/delete/rename/sweep
├─ ○ ≥60% files .md or docs/ ················ docs
├─ ○ ≥60% files under tests/ ················ test
├─ subject has ci/mise/docker/lint/deploy ··· ops
├─ ○ ≥60% files .css/.js/.html/static/ ······ web
├─ subject has add/new/wire/expose/ship ····· feature
└─ otherwise ································ other

FLOW · ● weekly throughput vs your own median
├─ ○ recent > 1.25× ··· ramping
├─ ○ recent < 0.70× ··· slowing → \"blocked, or shipping less?\"
└─ else ··············· steady

BATCH · ● lines/commit this week vs your median  (smaller = faster)
├─ ○ recent > 1.25× ··· rising  → \"split smaller — cheapest flow win\"
├─ ○ recent < 0.80× ··· easing
└─ else ··············· steady   (headline shows p__ = your percentile)

THRASH · ● in-place rewrite, aged by survival   (% of churn)
│    weight w = S(age)   ·   thrash = Σ w × rw
│    rw = min(added,deleted) ÷ deleted  per file  (1=rewrite, 0=removal)
├─ ○ < 8%  ··· low      → \"real throughput, not thrashing\"
├─ ○ < 15% ··· elevated → \"rename/format sweep? sanity-check\"
└─ ○ ≥ 15% ··· high     → \"stabilize this area before adding\"

EXCISION · ● the Σ w × (1−rw) half   (% of churn)
└─ always ···· healthy → \"deliberate scope-cutting\"

CADENCE · local time via `date +%z` · night = 20:00–05:59 · weekend = Sat/Sun
├─ ○ recent night% > baseline + 7 ····· nights ↑ → \"protect rest\"   (drift)
├─ ○ weekends > 35%  or  nights > 25% ·· heavy   → \"protect recovery\" (level)
└─ else ······························· steady

VERDICT · the top line (composed, never an LLM)
├─ ○ < 3 weeks history → \"BUILDING BASELINE — provisional\"
├─ lead  = batch phrase + thrash phrase
└─ watch = any of: batch rising · nights ↑ · thrash ≥15%
           none → \"nothing drifting — you're building, not spinning\"

half-life · ● first age where survival S(age) ≤ 0.5
not inferred: deploys/incidents/lead-time · people · cross-repo ranks
";
    // ● self-calibrated (green) · ○ tunable (yellow) — no-ops when color is off
    print!(
        "{}",
        raw.replace('●', &p.green("●")).replace('○', &p.yellow("○"))
    );
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const REPORT_CSS: &str = "\
:root{--bg:#fafafa;--panel:#fff;--ink:#18181b;--muted:#71717a;--line:#e4e4e7;\
--calm:#52525b;--good:#15803d;--watch:#b45309;--alarm:#b91c1c;\
--calmbg:#f4f4f5;--goodbg:#f0fdf4;--watchbg:#fffbeb;--alarmbg:#fef2f2}\
@media(prefers-color-scheme:dark){:root{--bg:#09090b;--panel:#161618;--ink:#fafafa;\
--muted:#a1a1aa;--line:#27272a;--calm:#a1a1aa;--good:#4ade80;--watch:#fbbf24;--alarm:#f87171;\
--calmbg:#1d1d20;--goodbg:#0c1f14;--watchbg:#221a06;--alarmbg:#250d0d}}\
*{box-sizing:border-box}\
body{margin:0;background:var(--bg);color:var(--ink);\
font:15px/1.55 -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif}\
.wrap{max-width:720px;margin:0 auto;padding:2.5rem 1.25rem 4rem}\
h1{font-size:1rem;font-weight:600;margin:0;letter-spacing:.01em}\
.meta{color:var(--muted);font-size:.85rem;margin-top:.15rem}\
.verdict{margin:1.4rem 0;padding:.95rem 1.15rem;background:var(--panel);border:1px solid var(--line);\
border-left:4px solid var(--calm);border-radius:10px;font-weight:500}\
.verdict.good{border-left-color:var(--good)}\
.verdict.watch{border-left-color:var(--watch);background:var(--watchbg)}\
.verdict.alarm{border-left-color:var(--alarm);background:var(--alarmbg)}\
.grid{display:grid;gap:.7rem}\
.card{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:.85rem 1.1rem}\
.row{display:flex;align-items:baseline;gap:.6rem}\
.key{font-weight:600;min-width:5rem}\
.chip{font-size:.72rem;font-weight:600;padding:.12rem .5rem;border-radius:999px;color:var(--calm);background:var(--calmbg)}\
.chip.good{color:var(--good);background:var(--goodbg)}\
.chip.watch{color:var(--watch);background:var(--watchbg)}\
.chip.alarm{color:var(--alarm);background:var(--alarmbg)}\
.head{margin-left:auto;color:var(--muted);font-size:.9rem;font-variant-numeric:tabular-nums}\
.spark{display:flex;align-items:flex-end;gap:2px;height:30px;margin-top:.6rem}\
.bar{width:9px;border-radius:2px 2px 0 0;background:var(--calm);opacity:.3}\
.card.good .bar{background:var(--good)}.card.watch .bar{background:var(--watch)}.card.alarm .bar{background:var(--alarm)}\
.bar.now{opacity:1}\
.note{color:var(--muted);font-size:.85rem;margin-top:.55rem}\
footer{margin-top:1.6rem;border-top:1px solid var(--line);padding-top:1rem;color:var(--muted);font-size:.82rem}\
.safety{margin-top:.5rem;font-size:.77rem}\
.section{margin-top:1.9rem}\
.section h2{font-size:.92rem;font-weight:600;margin:0 0 .15rem;letter-spacing:.01em}\
.sub{color:var(--muted);font-size:.8rem;margin:0 0 .7rem}\
.panel{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:.7rem 1rem}\
.empty{color:var(--muted);font-size:.85rem;margin:.2rem 0}\
.trow{display:flex;align-items:center;gap:.55rem;padding:.16rem 0;font-size:.88rem;font-variant-numeric:tabular-nums}\
.tbar,.hbar{flex:0 0 84px;height:7px;background:var(--line);border-radius:4px;overflow:hidden}\
.tbar i,.hbar i{display:block;height:100%;border-radius:4px;background:var(--calm)}\
.hbar i{background:var(--watch)}\
.tbar.good i{background:var(--good)}.tbar.watch i{background:var(--watch)}.tbar.alarm i{background:var(--alarm)}\
.tname{flex:1 1 auto;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}\
.tnum{color:var(--ink);min-width:3.6rem;text-align:right;font-weight:500}\
.tpct{color:var(--muted);min-width:2.8rem;text-align:right}\
.traj{width:1rem;text-align:center;font-weight:700}\
.traj.up{color:var(--watch)}.traj.down{color:var(--good)}.traj.flat{color:var(--muted)}\
.hrow{display:flex;align-items:center;gap:.6rem;padding:.18rem 0;font-size:.88rem;font-variant-numeric:tabular-nums}\
.hfile{flex:1 1 auto;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;\
font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.82rem}\
.hmeta{color:var(--muted);font-size:.8rem;white-space:nowrap}\
.srow{display:flex;align-items:center;gap:.8rem;padding:.3rem 0}\
.sname{flex:0 0 9rem;font-size:.85rem;font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}\
.scurve{flex:1 1 auto;height:44px;width:100%;display:block}\
.scurve .area{fill:var(--calm);opacity:.14}\
.scurve .line{fill:none;stroke:var(--calm);stroke-width:1.4;vector-effect:non-scaling-stroke}\
.scurve .mid{stroke:var(--line);stroke-width:1;stroke-dasharray:3 3;vector-effect:non-scaling-stroke}\
.smeta{flex:0 0 auto;color:var(--muted);font-size:.82rem;white-space:nowrap;font-variant-numeric:tabular-nums}\
.hgrid{display:grid;grid-template-columns:2.4rem repeat(24,1fr);gap:2px;align-items:center}\
.hhour{font-size:.62rem;color:var(--muted);grid-row:1;overflow:visible;white-space:nowrap}\
.hday{font-size:.72rem;color:var(--muted);padding-right:.4rem;white-space:nowrap}\
.hgrid i.cell{height:13px;border-radius:2px;background:var(--good);display:block}\
.hgrid i.cell.peak{outline:1.5px solid var(--ink);outline-offset:1px}";

/// Self-contained HTML report (the `report` command / `--report`): one web page
/// carrying the same semantics as all three terminal views — the verdict-first
/// cockpit, the thrash folder tree, and the hotspots list. No external assets
/// (inline CSS, no CDN/JS), so it opens offline and emails cleanly.
pub fn write_report(
    c: &Cockpit,
    tree: &TreeNode,
    hotspots: &[Hotspot],
    heat: &Heatmap,
    recent: bool,
    multi: bool,
    path: &str,
) -> Result<(), String> {
    let worst = c
        .cards
        .iter()
        .map(|x| x.tone)
        .max_by_key(|t| t.rank())
        .unwrap_or(Tone::Calm);

    let mut cards = String::new();
    for card in &c.cards {
        let t = tone_class(card.tone);
        let note = card
            .note
            .as_ref()
            .map(|n| format!("<div class=note>{}</div>", esc(n)))
            .unwrap_or_default();
        cards.push_str(&format!(
            "<div class=\"card {t}\"><div class=row>\
               <span class=key>{key}</span>\
               <span class=\"chip {t}\">{state}</span>\
               <span class=head>{head}</span></div>\
             <div class=spark>{bars}</div>{note}</div>",
            key = esc(&card.key),
            state = esc(&card.state),
            head = esc(&card.headline),
            bars = spark_bars(&card.spark_values),
        ));
    }

    let window = if recent { "last 8 weeks" } else { "all-time" };
    let thr_sub = if recent {
        format!(
            "In-place rewrite, weighted by recency, by folder · {window}. \
             % = thrash as a share of that folder's churn. \
             ↑ heating / ↓ cooling vs the 8-week pace."
        )
    } else {
        format!(
            "In-place rewrite, weighted by recency, by folder · {window}. \
             % = thrash as a share of that folder's churn."
        )
    };
    let hot_sub = format!(
        "Changed often AND deeply nested — the highest-ROI refactor targets · {window}. \
         Nx = commits that touched the file · cx = nesting complexity."
    );
    let (surv_h2, surv_sub) = if c.personal {
        (
            "my code survival — S(age)",
            "How long the lines you write survive — your own line-lifetime curve. The dashed \
             line is 50%; where the curve crosses it is the half-life you'd expect of your code. \
             Fit per repo.",
        )
    } else {
        (
            "code survival — S(age)",
            "Every deleted line is weighted by S(age) — its odds of having lived this long, read \
             off the repo's own line-lifetime curve. The dashed line is 50%; where the curve \
             crosses it is the half-life. Fit per repo.",
        )
    };
    let cad_sub = format!(
        "When commits land, by weekday and hour ({}, all history). \
         Darker = busier; peak {} {:02}:00.",
        esc(&heat.tz),
        DAYS[heat.peak_day],
        heat.peak_hour,
    );

    let html = format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8>\
<meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>Terminal Velocity · {branch}</title><style>{css}</style></head>\
<body><main class=wrap>\
<header><h1>terminal velocity</h1><div class=meta>{branch} · {window_lbl}</div></header>\
<section class=\"verdict {mood}\">{verdict}</section>\
<section class=section><h2>{surv_h2}</h2>\
<p class=sub>{surv_sub}</p><div class=panel>{survival}</div></section>\
<section class=grid>{cards}</section>\
<section class=section><h2>cadence — when commits land</h2>\
<p class=sub>{cad_sub}</p><div class=panel>{cadence}</div></section>\
<section class=section><h2>thrash — in-place rewrite</h2>\
<p class=sub>{thr_sub}</p><div class=panel>{thrash}</div></section>\
<section class=section><h2>hotspots — refactor targets</h2>\
<p class=sub>{hot_sub}</p><div class=panel>{hot}</div></section>\
<footer>{footer}<div class=safety>Self-relative: thresholds are percentiles \
against this repo's own history, not external benchmarks.</div></footer>\
</main></body></html>",
        branch = esc(&c.branch),
        window_lbl = esc(&c.window),
        mood = tone_class(worst),
        verdict = esc(&c.verdict),
        footer = esc(&c.footer),
        thr_sub = esc(&thr_sub),
        hot_sub = esc(&hot_sub),
        surv_h2 = esc(surv_h2),
        surv_sub = esc(surv_sub),
        cad_sub = esc(&cad_sub),
        survival = report_survival(&c.survival),
        cadence = report_heatmap(heat),
        thrash = report_thrash(tree, recent),
        hot = report_hotspots(hotspots, multi),
        css = REPORT_CSS,
    );

    fs::write(path, html).map_err(|e| format!("failed to write {path}: {e}"))?;
    Ok(())
}

/// The cadence punchcard as a CSS grid — opacity scales with commit count, the
/// busiest cell outlined; each cell carries a hover title (day · hour · count).
fn report_heatmap(h: &Heatmap) -> String {
    if h.total == 0 {
        return "<p class=empty>(no commits)</p>".to_string();
    }
    let mut g = String::from("<div class=hgrid><span class=hcorner></span>");
    for hh in 0..24 {
        let lbl = if hh % 3 == 0 {
            hh.to_string()
        } else {
            String::new()
        };
        g.push_str(&format!("<span class=hhour>{lbl}</span>"));
    }
    for (d, row) in h.counts.iter().enumerate() {
        g.push_str(&format!("<span class=hday>{}</span>", DAYS[d]));
        for (hh, &c) in row.iter().enumerate() {
            let op = if c == 0 {
                0.0
            } else {
                0.12 + 0.88 * (c as f64 / h.max as f64)
            };
            let peak = if d == h.peak_day && hh == h.peak_hour {
                " peak"
            } else {
                ""
            };
            g.push_str(&format!(
                "<i class=\"cell{peak}\" style=\"opacity:{op:.2}\" title=\"{} {hh:02}:00 · {c}\"></i>",
                DAYS[d],
            ));
        }
    }
    g.push_str("</div>");
    g
}

/// The survival curve(s) as HTML rows — an SVG area chart per repo, with the
/// half-life and alive-at-HEAD fraction alongside.
fn report_survival(survivals: &[RepoSurvival]) -> String {
    if survivals.is_empty() {
        return "<p class=empty>(no survival data)</p>".to_string();
    }
    let multi = survivals.len() > 1;
    let mut out = String::new();
    for s in survivals {
        let name = if multi {
            format!("<span class=sname>{}</span>", esc(&s.label))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<div class=srow>{name}{svg}\
               <span class=smeta>half-life {hl} · {alive:.0}% alive</span></div>",
            svg = survival_svg(&s.curve),
            hl = esc(&s.half_life),
            alive = s.alive_pct,
        ));
    }
    out
}

/// S(age) as an SVG area chart: y = survival (1 at top), x = age. A dashed 50%
/// gridline so the half-life crossing is visible. Stretches to its container.
fn survival_svg(curve: &[f64]) -> String {
    if curve.len() < 2 {
        return "<span class=smeta>(no deaths in range)</span>".to_string();
    }
    let (w, h) = (100.0_f64, 32.0_f64);
    let n = curve.len();
    let pt = |i: usize| -> (f64, f64) {
        let x = i as f64 / (n - 1) as f64 * w;
        let y = (1.0 - curve[i].clamp(0.0, 1.0)) * h;
        (x, y)
    };
    let (x0, y0) = pt(0);
    let mut line = format!("M{x0:.1},{y0:.1}");
    for i in 1..n {
        let (x, y) = pt(i);
        line.push_str(&format!(" L{x:.1},{y:.1}"));
    }
    let area = format!("{line} L{w:.0},{h:.0} L0,{h:.0} Z");
    format!(
        "<svg class=scurve viewBox=\"0 0 {w:.0} {h:.0}\" preserveAspectRatio=none>\
           <line class=mid x1=0 y1=\"{mid:.0}\" x2=\"{w:.0}\" y2=\"{mid:.0}\"/>\
           <path class=area d=\"{area}\"/><path class=line d=\"{line}\"/></svg>",
        mid = h / 2.0,
    )
}

/// The thrash folder tree as HTML rows — same prune/scale/sort as the terminal.
fn report_thrash(tree: &TreeNode, recent: bool) -> String {
    if tree.thrash <= 0.0 || tree.children.is_empty() {
        return "<p class=empty>(no rework recorded)</p>".to_string();
    }
    let min = (tree.thrash * 0.025).max(1.0);
    let scale = tree
        .children
        .values()
        .map(|c| c.thrash)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let mut out = String::new();
    let mut shown = 0usize;
    report_branch(tree, scale, min, 0, recent, &mut shown, &mut out);
    out
}

fn report_branch(
    node: &TreeNode,
    scale: f64,
    min: f64,
    depth: usize,
    recent: bool,
    shown: &mut usize,
    out: &mut String,
) {
    let mut kids: Vec<(&String, &TreeNode)> = node
        .children
        .iter()
        .filter(|(_, c)| c.thrash >= min)
        .collect();
    kids.sort_by(|a, b| {
        b.1.thrash
            .partial_cmp(&a.1.thrash)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (name, child) in kids {
        if *shown >= 40 {
            return;
        }
        *shown += 1;
        let pct = if child.churn > 0.0 {
            child.thrash / child.churn * 100.0
        } else {
            0.0
        };
        let t = tone_class(thr_tone(pct));
        let w = (child.thrash / scale * 100.0).clamp(2.0, 100.0);
        out.push_str(&format!(
            "<div class=trow>\
               <span class=\"tbar {t}\"><i style=\"width:{w:.0}%\"></i></span>\
               <span class=tname style=\"padding-left:{pad:.2}rem\">{name}</span>{traj}\
               <span class=tnum>{thr:.0}</span><span class=tpct>{pct:.0}%</span></div>",
            pad = depth as f64 * 1.1,
            name = esc(name),
            traj = report_traj(child, recent),
            thr = child.thrash,
        ));
        report_branch(child, scale, min, depth + 1, recent, shown, out);
    }
}

/// Trajectory glyph for a folder (last-7d vs the 8-week pace), only when windowed.
fn report_traj(node: &TreeNode, recent: bool) -> String {
    if !recent {
        return String::new();
    }
    let r = if node.thrash > 0.0 {
        node.thrash_recent / node.thrash
    } else {
        0.0
    };
    let (cls, ch) = if r > 0.20 {
        ("up", "↑")
    } else if r < 0.06 {
        ("down", "↓")
    } else {
        ("flat", "→")
    };
    format!("<span class=\"traj {cls}\">{ch}</span>")
}

/// The hotspots list as HTML rows — sqrt-scaled bars, repo tag when aggregating.
fn report_hotspots(rows: &[Hotspot], multi: bool) -> String {
    if rows.is_empty() {
        return "<p class=empty>(no files)</p>".to_string();
    }
    let max = rows
        .iter()
        .map(|r| r.score)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let mut out = String::new();
    for r in rows {
        let w = ((r.score / max).sqrt() * 100.0).clamp(2.0, 100.0);
        let meta = if multi {
            format!("{}× · cx {} · {}", r.freq, r.complexity, esc(&r.repo))
        } else {
            format!("{}× · cx {}", r.freq, r.complexity)
        };
        out.push_str(&format!(
            "<div class=hrow>\
               <span class=hbar><i style=\"width:{w:.0}%\"></i></span>\
               <span class=hfile>{file}</span><span class=hmeta>{meta}</span></div>",
            file = esc(&r.file),
        ));
    }
    out
}

fn tone_class(t: Tone) -> &'static str {
    match t {
        Tone::Calm => "calm",
        Tone::Good => "good",
        Tone::Watch => "watch",
        Tone::Alarm => "alarm",
    }
}

/// Sparkline as a flex row of <div> bars; the final one (this week) is `.now`
/// (full opacity), matching the terminal's bold last bar.
fn spark_bars(vals: &[f64]) -> String {
    if vals.is_empty() {
        return String::new();
    }
    let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    let last = vals.len() - 1;
    let mut s = String::new();
    for (i, v) in vals.iter().enumerate() {
        let h = 14.0 + (v - min) / span * 86.0;
        let now = if i == last { " now" } else { "" };
        s.push_str(&format!(
            "<div class=\"bar{now}\" style=\"height:{h:.0}%\"></div>"
        ));
    }
    s
}
