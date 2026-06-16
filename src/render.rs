//! Output skins. Terminal cockpit is the default (daily glance); the HTML
//! report (the `report` command / `--report`) is the manager/retro skin —
//! cockpit + thrash + hotspots on one page. No composed verdict on either: the
//! board is git-status-shaped — every metric shown, each tagged with its status.

use std::fs;

use crate::metrics::{
    pareto_count, Hotspot, TreeNode, THRASH_ELEVATED_PCT, THRASH_HIGH_PCT, VITAL_FEW,
};
use crate::model::{Card, Cockpit, Heatmap, RepoSurvival, Tone};
use crate::spark::sparkline;
use crate::style::Palette;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Naive word-wrap (byte-width; fine for the mostly-ASCII footer).
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

pub fn print_cockpit(c: &Cockpit, p: &Palette, explain: bool) {
    let dim_rule = p.dim(&p.rule());

    // Masthead: the v∞ mark (terminal velocity = the velocity in the limit) on its
    // own line, the wordmark spelled out below. A quiet top-left logo that also
    // sets the board off from the shell prompt above it.
    println!("{}", p.dim("v∞"));

    // Header. Coverage honesty is the only survivor of the old composed verdict:
    // a caveat that applies to the whole board equally, so it rides the header
    // (yellow, to draw the eye), not any one metric.
    let provisional = if c.is_provisional() {
        format!(
            " {}",
            p.yellow(&format!("· provisional ({}wk)", c.coverage_weeks))
        )
    } else {
        String::new()
    };
    println!(
        "{} {}{provisional}",
        p.bold("terminal velocity"),
        p.dim(&format!("· {} · {}", c.branch, c.window)),
    );
    // `--explain` expands the header's "lately" right here — the recency lens frames
    // every card below, so it leads rather than hiding in the survival section.
    if explain {
        print_recency(p);
    }
    println!("{dim_rule}");
    println!();

    // No composed verdict: the gutter glyph on each card is the status, git-status
    // style. The reader's eye triages; the tool never editorializes what matters.
    // `--explain` expands each metric in place into its own decision tree, with the
    // branch that fired for this repo lit — the explanation lives where the metric
    // does, never as a wall appended after the board.
    for (i, card) in c.cards.iter().enumerate() {
        if explain && i > 0 {
            println!(); // breathing room between expanded metric blocks
        }
        print_card(card, p);
        if explain {
            print_card_explain(card, p);
        }
    }
    println!();
    println!("{dim_rule}");
    println!();

    // Survival sits below the indicators, glyph-less — the foundation they weight
    // against, not a graded metric of its own. `--explain` expands its one-line
    // gloss into the S(age) formula + the prose story (it flows from the bottom
    // line: the curve, then how it's read, then what it means).
    if !c.survival.is_empty() {
        print_survival(&c.survival, c.personal, explain, p);
        println!();
        println!("{dim_rule}");
        println!();
    }

    for line in wrap(&c.footer, p.width) {
        println!("{}", p.dim(&line));
    }

    // The deliberate non-goals — what tv refuses to infer from git alone — plus the
    // glyph/marker legend the trees above rely on (the retired `tv explain`'s one job).
    if explain {
        println!(
            "{}",
            p.dim("glyphs · · calm  ✓ good  ▲ watch  ■ alarm   ·   ● self-calibrated  ○ tunable")
        );
        println!(
            "{}",
            p.dim("not inferred: deploys / incidents / lead-time · people · cross-repo ranks")
        );
    }
}

/// `--explain`: expand a card in place into its decision tree, the branch that
/// fired (this repo's current state) lit and its siblings dimmed. Slices this metric's
/// section out of `TREE_BODY`; the report's `report_card_explain` reads the same source,
/// so the terminal and HTML surfaces never drift.
fn print_card_explain(card: &Card, p: &Palette) {
    let Some(block) = tree_block(&card.key.to_uppercase()) else {
        return;
    };
    for (i, line) in block.lines().enumerate() {
        let trimmed = line.trim_start();
        let is_branch = trimmed.starts_with("├─") || trimmed.starts_with("└─");
        let styled = if is_branch && line.contains(card.state.as_str()) {
            p.tone(card.tone, &p.bold(line)) // the branch you're on
        } else if is_branch {
            p.dim(line) // a road not taken
        } else if i == 0 {
            // section header → drop the "TITLE · " prefix, keep the definition
            p.dim(line.split_once(" · ").map_or(line, |(_, def)| def))
        } else {
            p.dim(line) // a continuation/formula line (e.g. thrash's weighting)
        };
        println!("     {styled}");
    }
    // The edge: what feeds this metric (today, only thrash ← intent). Drawing it
    // here is where `intent` stops being a dead-end in the explain view.
    if let Some(detail) = &card.detail {
        println!("     {}", p.dim(&format!("→ {detail}")));
    }
}

/// The `TREE_BODY` block (header + branches) for a metric section, e.g. "FLOW".
fn tree_block(title: &str) -> Option<&'static str> {
    TREE_BODY
        .split("\n\n")
        .find(|b| b.split([' ', '·']).next() == Some(title))
}

fn print_card(card: &Card, p: &Palette) {
    // The left-gutter glyph is the at-a-glance status; reading the column down
    // the board is the whole "verdict". Symbol carries it; color reinforces.
    let glyph = p.tone(card.tone, card.tone.glyph());
    let key = p.bold(&format!("{:<9}", card.key));
    let spark = decorate_spark(p, card.tone, &card.spark);
    let state = p.tone(card.tone, &card.state);
    if card.available {
        println!("  {glyph}  {key} {spark} {state} · {}", card.headline);
    } else {
        println!("  {glyph}  {key} {spark} {state}");
    }
    if let Some(note) = &card.note {
        println!("{}", p.dim(&format!("        └ {note}")));
    }
}

/// The survival curve(s) — S(age) — that weight every thrash/excision, shown
/// below the indicators as their foundation. One repo: curve + half-life +
/// alive%, with a one-line gloss. Several: one compact row per repo (fit per repo).
fn print_survival(survivals: &[RepoSurvival], personal: bool, explain: bool, p: &Palette) {
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
            p.dim(&format!("{:.0}% alive", s.alive_pct)),
        );
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

    // The gloss: `--explain` gives the full S(age) formula + story; otherwise the
    // one-line read (single repo only — the multi-repo rows speak for themselves).
    if explain {
        print_survival_formula(p);
    } else if survivals.len() == 1 {
        let gloss = if personal {
            "how long the lines you write survive · `--explain` to unpack it"
        } else {
            "how long code survives here, fit to this repo · `--explain` to unpack it"
        };
        for line in wrap(gloss, p.width - 2) {
            println!("{}", p.dim(&format!("  {line}")));
        }
    }
}

/// The S(age) story — the mental model, in prose, told once in full (this is the
/// verbose surface; `--explain` is where length is welcome). One paragraph per beat.
const SURVIVAL_STORY: &[&str] = &[
    "Every line is born the moment a commit first writes it. From then it ages on \
     two clocks at once: one in days, one in commits, a pulse you can read in either.",
    "A line can meet two fates. It can be excised, cut from the tree and gone. That \
     is a death, the only kind that counts. Or it can be rewritten in place, reshaped \
     but still standing. That isn't dying. It's aging, and the line lives on. So do \
     the lines still here at HEAD. We don't know their ending yet, so we don't \
     pretend to.",
    "From the deaths alone, the repo draws its own survival curve. No borrowed \
     thirty-day rule, no calendar but its own. At each age a line dies, it asks one \
     question: of those that made it this far, what share slip away here? Chain those \
     odds together and you have S(t), the chance a line outlives age t.",
    "Halfway down sits the half-life, the age where a line's odds of still being here \
     fall to fifty-fifty. Maybe 103 days, maybe 503 commits, one heartbeat told two \
     ways. In a young or stubborn repo the curve never falls that far. More than half \
     the lines simply refuse to die, and the half-life is reported, honestly, as not \
     yet reached.",
    "Then there's the elder: one legendary line from a founding commit, survivor of \
     every excision since, still standing in the code today. Most of its cohort \
     didn't make it. It hasn't won. It's only still alive, its age still counting, \
     its ending still unwritten.",
    "The story of your code is written in git, one line at a time. tv only reads it \
     back to you.",
];

/// The `--explain` survival block: the Kaplan-Meier product-limit formula, stacked
/// (alignment is load-bearing — printed verbatim), a per-symbol breakdown, the edge
/// it feeds, and then the full story. Replaces the one-line gloss.
fn print_survival_formula(p: &Palette) {
    println!();
    for line in [
        "           ⎛      dᵢ  ⎞",
        "  S(t) = ∏ ⎜ 1 − ──── ⎟",
        "       tᵢ≤t⎝      nᵢ  ⎠",
    ] {
        println!("{line}");
    }
    println!();
    for line in [
        "  dᵢ  deaths at age tᵢ    lines excised, not rewritten",
        "  nᵢ  still at risk        alive and not yet censored",
        "  t   age                  in commits, or days",
        "  →   feeds thrash weight w = S(age) · excision weight",
    ] {
        println!("{}", p.dim(line));
    }
    // The story is the narrative beat — signposted so the shift from the terse math
    // above is intentional, not a wall. (Recency lives up top now, by the header.)
    println!();
    println!("{}", p.dim("  the model, in plain words:"));
    for para in SURVIVAL_STORY {
        println!();
        for line in wrap(para, p.width - 2) {
            println!("{}", p.dim(&format!("  {line}")));
        }
    }
}

/// `--explain` only: unpack the header's "lately" — the recency lens every figure on
/// the board shares — right where it's introduced, before the cards that use it.
fn print_recency(p: &Palette) {
    println!(
        "{}",
        p.dim(&format!(
            "a commit's pull by age, vs one made today (½ every ~{:.0}d):",
            crate::metrics::RECENCY_HALFLIFE_DAYS
        ))
    );
    println!(
        "{}",
        p.dim(&format!("  {}", crate::metrics::recency_anchors()))
    );
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
    if pct < THRASH_ELEVATED_PCT {
        Tone::Good
    } else if pct < THRASH_HIGH_PCT {
        Tone::Watch
    } else {
        Tone::Alarm
    }
}

/// The drill-downs' basis label — the recency lens (so thrash & hotspots reconcile with
/// `tv status`), or flat lifetime totals under `--all` (`recent=false`). When anchored
/// (`as_of` set) it drops the now-implying "lately" and names the date instead. Both
/// surfaces (terminal + report) read this one label.
fn lens_window(recent: bool, as_of: Option<&str>) -> String {
    match (recent, as_of) {
        (true, None) => "lately · recency-weighted".to_string(),
        (true, Some(d)) => format!("recency-weighted, to {d}"),
        (false, None) => "all-time · unweighted".to_string(),
        (false, Some(d)) => format!("all-time, to {d}"),
    }
}

pub fn print_thrash(branch: &str, root: &TreeNode, recent: bool, as_of: Option<&str>, p: &Palette) {
    // Same recency lens as `tv status` (so the root reconciles with the thrash card);
    // `--all` (recent=false) switches to flat lifetime totals.
    let window = lens_window(recent, as_of);
    // Root total = the cockpit thrash card's number (same lens) — shown so the per-folder
    // intensities below (each a share of *its own* churn) don't read as non-reconciling.
    let overall = root.thrash_pct();
    println!("{}", p.dim("v∞"));
    println!(
        "{} {}",
        p.bold("tv thrash"),
        p.dim(&format!("· {branch} · {window} · ~{overall:.1}% of churn"))
    );
    println!("{}", p.dim(&p.rule()));
    println!(
        "{}",
        p.dim("in-place rewrite: recently-written code rewritten again, by folder.")
    );
    println!(
        "{}",
        p.dim("% = thrash as a share of that folder's churn — same lens as `tv status`.")
    );
    println!(
        "{}",
        p.dim("shows the folders that carry 80% of the rework — the vital few.")
    );
    if recent {
        // "last" only when the window ends now; anchored, it's the 7d before the anchor.
        let traj = if as_of.is_some() {
            "↑ heating / ↓ cooling = 7d vs your recent pace."
        } else {
            "↑ heating / ↓ cooling = last 7d vs your recent pace."
        };
        println!("{}", p.dim(traj));
    }
    println!();
    if root.thrash <= 0.0 || root.children.is_empty() {
        println!("  (no rework recorded)");
        return;
    }
    let scale = root
        .children
        .values()
        .map(|c| c.thrash)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let floor = thrash_floor(root);
    print_branch(p, root, scale, floor, "", recent);

    println!("{}", p.dim(&p.rule()));
    println!(
        "{}",
        p.dim(&format!(
            "total thrash {:.0} · excision {:.0} (deliberate removals, not rework)",
            root.thrash, root.excision
        ))
    );
}

/// A folder's recent rework trajectory: last-7d thrash as a fraction of its (recency-
/// weighted) total thrash, bucketed by empirical cut-points — how much of the recent-
/// leaning rework landed in the last 7 days. The cut lives here once so the terminal
/// arrow and the HTML glyph can never disagree.
enum Trend {
    Heating, // ↑ more recent rework than its steady share
    Cooling, // ↓ cooled off
    Steady,  // → roughly on pace
}

const TRAJ_HEATING: f64 = 0.20;
const TRAJ_COOLING: f64 = 0.06;

fn trend(node: &TreeNode) -> Trend {
    let r = if node.thrash > 0.0 {
        node.thrash_recent / node.thrash
    } else {
        0.0
    };
    if r > TRAJ_HEATING {
        Trend::Heating
    } else if r < TRAJ_COOLING {
        Trend::Cooling
    } else {
        Trend::Steady
    }
}

/// Trend arrow for a folder (terminal). A space when not in the windowed view.
fn traj_arrow(p: &Palette, recent: bool, node: &TreeNode) -> String {
    if !recent {
        return " ".to_string();
    }
    match trend(node) {
        Trend::Heating => p.yellow("↑"),
        Trend::Cooling => p.green("↓"),
        Trend::Steady => p.dim("→"),
    }
}

/// Each folder's *own* (non-inherited) rework = its thrash minus its children's.
/// These partition the tree's total exactly, so collecting them lets one Pareto
/// cut set a single global significance floor.
fn own_contributions(node: &TreeNode, out: &mut Vec<f64>) {
    let kids: f64 = node.children.values().map(|c| c.thrash).sum();
    let own = node.thrash - kids;
    if own > 0.0 {
        out.push(own);
    }
    for c in node.children.values() {
        own_contributions(c, out);
    }
}

/// The single, self-calibrating tree limiter: the rework level below which a
/// folder isn't worth showing. Set so the visible folders carry ~80% of the total
/// rework (Pareto over per-folder own-contributions) — replaces the old 2.5%
/// threshold + 40-node cap. Floored at 1.0 so trivial nodes never show.
fn thrash_floor(root: &TreeNode) -> f64 {
    let mut owns = Vec::new();
    own_contributions(root, &mut owns);
    owns.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let k = pareto_count(&owns, VITAL_FEW);
    if k == 0 {
        f64::INFINITY
    } else {
        owns[k - 1].max(1.0)
    }
}

/// A node's children worth showing (subtree ≥ `floor`), biggest-thrash first.
fn kept_children(node: &TreeNode, floor: f64) -> Vec<(&String, &TreeNode)> {
    let mut kids: Vec<(&String, &TreeNode)> = node
        .children
        .iter()
        .filter(|(_, c)| c.thrash >= floor)
        .collect();
    kids.sort_by(|a, b| {
        b.1.thrash
            .partial_cmp(&a.1.thrash)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    kids
}

/// Print a folder's children as an indented tree, biggest first, above `floor`.
fn print_branch(p: &Palette, node: &TreeNode, scale: f64, floor: f64, prefix: &str, recent: bool) {
    let kids = kept_children(node, floor);
    let n = kids.len();
    for (i, (name, child)) in kids.iter().enumerate() {
        let last = i == n - 1;
        let connector = if last { "└─ " } else { "├─ " };
        let pct = child.thrash_pct();
        let label = trunc(&format!("{prefix}{connector}{name}"), 30);
        println!(
            "  {label} {} {}  {}  {}",
            hbar(p, child.thrash / scale, 8, thr_tone(pct)),
            traj_arrow(p, recent, child),
            p.bold(&format!("{:>5.0}", child.thrash)),
            p.dim(&format!("· {pct:.0}%")),
        );
        let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
        print_branch(p, child, scale, floor, &child_prefix, recent);
    }
}

pub fn print_hotspots(
    scope: &str,
    rows: &[Hotspot],
    recent: bool,
    multi: bool,
    as_of: Option<&str>,
    p: &Palette,
) {
    let window = lens_window(recent, as_of);
    println!("{}", p.dim("v∞"));
    println!(
        "{} {}",
        p.bold("tv hotspots"),
        p.dim(&format!(
            "· {scope} · {window} · revisions × complexity × recency"
        ))
    );
    println!("{}", p.dim(&p.rule()));
    println!(
        "{}",
        p.dim("files changed often AND deeply nested AND recently — top refactor ROI.")
    );
    if rows.is_empty() {
        println!();
        println!("  (no files)");
        return;
    }
    // Self-scaling cut: the vital few that carry 80% of the heat — no magic top-N.
    let scores: Vec<f64> = rows.iter().map(|r| r.score).collect();
    let cut = pareto_count(&scores, VITAL_FEW).max(1);
    println!(
        "{}",
        p.dim(&format!(
            "the 80% that carries the heat: {cut} of {} · Nx = commits · cx = nesting · rcy = how recent (1=today).",
            rows.len()
        ))
    );
    println!();
    let max = rows[0].score.max(1.0); // sorted desc → the hottest scales the bars
    for r in &rows[..cut] {
        // sqrt scaling so the list past the top file stays readable; the numbers
        // carry the real magnitude.
        let stats = if multi {
            format!(
                "{}× · cx {} · rcy {:.2} · {}",
                r.freq, r.complexity, r.recency, r.repo
            )
        } else {
            format!("{}× · cx {} · rcy {:.2}", r.freq, r.complexity, r.recency)
        };
        println!(
            "  {}  {} {}",
            hbar(p, (r.score / max).sqrt(), 10, Tone::Watch),
            trunc(&r.file, 38),
            p.dim(&stats),
        );
    }
    if rows.len() > cut {
        println!(
            "{}",
            p.dim(&format!("  +{} more below the cut", rows.len() - cut))
        );
    }
}

/// One rhythm-grid cell: shaded block scaled to the busiest cell (GitHub-green).
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

/// The single source of the shift heating/cooling thresholds: a margin's z-score → its
/// glyph, or `None` when steady. ▲ strong / ▴ mild heating, ▾ / ▼ cooling. Sample-size
/// aware (z = standardized residual vs the margin's own usual rate). Both surfaces (the
/// terminal gutter + the report axes), the report's CSS class, and the `any_*` gates all
/// derive from this, so the buckets can't drift. Monochrome — direction is the message,
/// not good/bad (the cockpit card owns the verdict).
/// Shift z-score cut-points (standardized residuals): a margin past STRONG marks strong
/// heating/cooling, past MILD a mild one. Named rather than inline per the "no arbitrary
/// numeric defaults" rule — siblings of metrics' `THRASH_*` / cadence-heavy cuts.
const SHIFT_STRONG_SIGMA: f64 = 3.0;
const SHIFT_MILD_SIGMA: f64 = 1.5;

fn shift_glyph(z: f64) -> Option<&'static str> {
    if z > SHIFT_STRONG_SIGMA {
        Some("▲")
    } else if z > SHIFT_MILD_SIGMA {
        Some("▴")
    } else if z < -SHIFT_STRONG_SIGMA {
        Some("▼")
    } else if z < -SHIFT_MILD_SIGMA {
        Some("▾")
    } else {
        None
    }
}

/// The cadence drill-down: a weekday × hour commit punchcard (local time). The grid is
/// the all-time *rhythm* (where you commit); the *shift* — how that rhythm is vectoring
/// lately — rides the grid's own axes, a ▴/▾ next to a weekday or above an hour marking
/// the days and hours that are heating or cooling. Rhythm is the static, shift the dynamic.
pub fn print_heatmap(h: &Heatmap, scope: &str, as_of: Option<&str>, p: &Palette) {
    let basis = match as_of {
        Some(d) => format!("all history thru {d}"),
        None => "all history".to_string(),
    };
    println!("{}", p.dim("v∞"));
    println!(
        "{} {}",
        p.bold("tv cadence"),
        p.dim(&format!(
            "· {scope} · when commits land · {} · {basis}",
            h.tz
        ))
    );
    println!("{}", p.dim(&p.rule()));
    if h.total == 0 {
        println!("  (no commits)");
        return;
    }
    // hour axis: 5-char day gutter, then a tick every 3 hours (2 cols each).
    let mut axis = String::from("     ");
    for hh in (0..24).step_by(3) {
        axis.push_str(&format!("{hh:<6}"));
    }

    // Hour-shift markers ride *above* the hour ticks, aligned over their columns
    // (2 cols/hour, matching the cells): a ▴/▾ over the hours heating/cooling lately.
    // Only drawn when one fires.
    let any_hour = h.hour_shift.iter().any(|&z| shift_glyph(z).is_some());
    let any_day = h.day_shift.iter().any(|&z| shift_glyph(z).is_some());
    if any_hour {
        let marks: String = (0..24)
            .map(|hh| format!("{} ", shift_glyph(h.hour_shift[hh]).unwrap_or(" ")))
            .collect();
        println!("     {marks}");
    }
    println!("{}", p.dim(&axis));

    // Rhythm grid: the all-time pattern. Each row is tagged in the gutter with its
    // weekday-shift marker (▴Mon = Mondays heating lately, ▾ cooling) — so the row and
    // column markers frame the same grid that shows the steady rhythm.
    for (d, row) in h.counts.iter().enumerate() {
        let cells: String = (0..24).map(|hh| heat_cell(p, row[hh], h.max)).collect();
        let mark = shift_glyph(h.day_shift[d]).unwrap_or(" ");
        println!("{mark}{} {cells}", p.bold(&format!("{:<3}", DAYS[d])));
    }
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
    // The shift legend, only when a marker actually fired; otherwise say so plainly, so
    // the dynamic is always reported (not just the static rhythm).
    if any_hour || any_day {
        println!(
            "{}",
            p.dim("  ▴ heating · ▾ cooling — a weekday (row) or hour (column) busier/quieter than its usual share, lately")
        );
    } else {
        println!(
            "{}",
            p.dim(
                "  lately: rhythm holding steady — no weekday or hour notably heating or cooling"
            )
        );
    }

    println!("{}", p.dim(&p.rule()));
    // Footer: the peak slot + the aggregate vector (reconciles with the status card).
    println!(
        "  {}  ·  {}",
        p.bold(&format!(
            "peak {} {:02}:00 ({} commits)",
            DAYS[h.peak_day], h.peak_hour, h.counts[h.peak_day][h.peak_hour]
        )),
        p.dim(&format!(
            "weekends {:.0}%→{:.0}% · nights {:.0}%→{:.0}% (all-time → lately)",
            h.weekend_all * 100.0,
            h.weekend_lately * 100.0,
            h.night_all * 100.0,
            h.night_lately * 100.0,
        ))
    );
}

/// Per-metric decision-tree sections, sliced by [`tree_block`] and shown (with the live
/// branch lit) under `tv status --explain`. `●` = self-calibrated, `○` = a tunable
/// constant; the glyph/marker legend rides the `--explain` footer (see [`print_cockpit`]).
/// Mirror of the logic in intent.rs / metrics.rs — keep in sync with thresholds.
const TREE_BODY: &str = "\
FLOW · ● this week's churn vs your recent-typical weekly churn
│    typical = recency-weighted median of prior weeks (recent weeks count most)
├─ ○ this week > 1.25× typical ··· ramping
├─ ○ this week < 0.70× typical ··· slowing → \"blocked, or shipping less?\"
└─ else ························· steady

BATCH · ● this week's lines/commit vs your recent-typical  (smaller = faster)
├─ ○ this week > 1.25× typical ··· rising → \"split smaller — cheapest flow win\"
├─ ○ this week < 0.80× typical ··· easing
└─ else ························· steady

THRASH · ● in-place rewrite, aged by survival × recency   (% of recent churn)
│    weight w = S(age) × recency(kill, ½-life 21d)   ·   thrash = Σ w × rw
│    rw = min(added,deleted) ÷ deleted  per file  (1=rewrite, 0=removal)
│    Wilson band [lo–hi] shown for confidence (n_eff = effective recent commits)
├─ ○ < 8%  ··· low      → \"real throughput, not thrashing\"
├─ ○ < 15% ··· elevated → \"rename/format sweep? sanity-check\"
└─ ○ ≥ 15% ··· high     → \"stabilize this area before adding\"

EXCISION · ● code removed outright (the Σ w × (1−rw) half) — thrash's twin
│    high = recent, normally-durable code pulled: cleanup, or false starts?
├─ ○ < 8%  ··· low     → \"little recent work pulled back out\"
├─ ○ < 15% ··· pruning → \"healthy scope-cutting\"
└─ ○ ≥ 15% ··· heavy   → \"decisive cleanup or false starts? git can't tell\"

CADENCE · recency-weighted (½-life 21d) · night 20:00–05:59 · weekend Sat/Sun (local)
├─ ○ weekend > 35% ··· heavy → \"protect recovery\"
├─ ○ night > 25% ····· heavy → \"protect recovery\"
└─ else ············· steady  (Wilson band in --explain; thin history → provisional)
";

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
.logo{color:var(--muted);font-size:.8rem;font-weight:600;letter-spacing:.02em}\
.meta{color:var(--muted);font-size:.85rem;margin-top:.15rem}\
.asof{display:inline-block;margin-top:.55rem;padding:.22rem .65rem;border-radius:999px;\
background:var(--watchbg);color:var(--watch);border:1px solid var(--watch);\
font-size:.8rem;font-weight:600;letter-spacing:.01em;font-variant-numeric:tabular-nums}\
.asof+.asof{margin-left:.4rem}\
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
details.card>summary{list-style:none;cursor:pointer;display:block}\
details.card>summary::-webkit-details-marker{display:none}\
.disc{margin-left:.5rem;color:var(--muted);font-size:.7rem}\
.disc::before{content:\"\u{25B8}\"}\
details.card[open] .disc::before{content:\"\u{25BE}\"}\
.tree{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;\
font-size:.74rem;line-height:1.55;color:var(--muted);margin:.75rem 0 0;white-space:pre-wrap;overflow-x:auto}\
.tree b{font-weight:700;color:var(--ink)}\
.card.good .tree b{color:var(--good)}.card.watch .tree b{color:var(--watch)}.card.alarm .tree b{color:var(--alarm)}\
.story{margin-top:1rem}\
.story p{font-size:.9rem;line-height:1.65;margin:0 0 .75rem}\
.lens{color:var(--muted);font-size:.82rem;margin:.2rem 0 0}\
details.formula{margin-top:.6rem}\
details.formula>summary{cursor:pointer;color:var(--muted);font-size:.8rem;font-weight:600}\
details.formula pre{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;\
font-size:.8rem;line-height:1.5;color:var(--ink);margin:.6rem 0 0;overflow-x:auto}\
footer{margin-top:1.6rem;border-top:1px solid var(--line);padding-top:1rem;color:var(--muted);font-size:.82rem}\
.safety{margin-top:.5rem;font-size:.77rem}\
.builtby{margin-top:.7rem}\
.builtby a{color:var(--ink);text-decoration:none;border-bottom:1px solid var(--line)}\
.builtby a:hover{border-color:var(--muted)}\
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
details.tnode>summary{list-style:none;cursor:pointer}\
details.tnode>summary::-webkit-details-marker{display:none}\
details.tnode>summary::marker{content:\"\"}\
.tw{display:inline-block;width:1.1em;color:var(--muted);font-size:.7em;vertical-align:1px}\
details.tnode>summary .tw::before{content:\"\u{25B8}\"}\
details.tnode[open]>summary .tw::before{content:\"\u{25BE}\"}\
summary.trow:hover{background:var(--calmbg);border-radius:6px}\
.hrow.more{color:var(--muted);font-size:.82rem;padding-left:.4rem;margin-top:.1rem}\
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
.hhour{font-size:.62rem;color:var(--muted);overflow:visible;white-space:nowrap}\
.hmark{font-size:.7rem;line-height:1;text-align:center;color:var(--muted)}\
.hday{font-size:.72rem;color:var(--muted);padding-right:.4rem;white-space:nowrap}\
.hgrid i.cell{height:13px;border-radius:2px;background:var(--good);display:block}\
.hgrid i.cell.peak{outline:1.5px solid var(--ink);outline-offset:1px}\
.hmark.heat,.hday.heat{color:#d9822b}.hmark.cool,.hday.cool{color:#4a90d9}\
.hday.heat,.hday.cool{font-weight:600}";

/// Self-contained HTML report (the `report` command / `--report`): one web page
/// carrying the same semantics as all three terminal views — the status-board
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
    let as_of = c.as_of.as_deref();

    let mut cards = String::new();
    for card in &c.cards {
        let t = tone_class(card.tone);
        let note = card
            .note
            .as_ref()
            .map(|n| format!("<div class=note>{}</div>", esc(n)))
            .unwrap_or_default();
        // The card is a <details>: the summary is the at-a-glance row + sparkline,
        // expanding to the metric's decision tree (fired branch lit) — the report's
        // take on `--explain`, here as native progressive disclosure.
        cards.push_str(&format!(
            "<details class=\"card {t}\"><summary><div class=row>\
               <span class=key>{key}</span>\
               <span class=\"chip {t}\">{state}</span>\
               <span class=head>{head}</span>\
               <span class=disc></span></div>\
             <div class=spark>{bars}</div>{note}</summary>{explain}</details>",
            key = esc(&card.key),
            state = esc(&card.state),
            head = esc(&card.headline),
            bars = spark_bars(&card.spark_values),
            explain = report_card_explain(card),
        ));
    }

    let thr_overall = tree.thrash_pct();
    let thr_window = format!(
        "{} · ~{thr_overall:.1}% of churn",
        lens_window(recent, as_of)
    );
    let thr_sub = if recent {
        format!(
            "In-place rewrite of recently-written code, by folder · {thr_window}. \
             % = thrash as a share of that folder's churn (same lens as the board). \
             ↑ heating / ↓ cooling = last 7d vs your recent pace."
        )
    } else {
        format!(
            "In-place rewrite of recently-written code, by folder · {thr_window}. \
             % = thrash as a share of that folder's churn (same lens as the board)."
        )
    };
    let hot_window = lens_window(recent, as_of);
    let hot_sub = format!(
        "Changed often AND deeply nested AND recently — the highest-ROI refactor targets · {hot_window}. \
         Nx = commits · cx = nesting complexity · rcy = how recent (1 = today)."
    );
    let (surv_h2, surv_sub) = if c.personal {
        (
            "my code survival — S(age)",
            "How long the lines you write survive — your own line-survival curve, fit per repo. \
             The dashed line is 50%; where the curve crosses it is the half-life.",
        )
    } else {
        (
            "code survival — S(age)",
            "How long code lives in this repo — its own line-survival curve, fit per repo. The \
             dashed line is 50%; where the curve crosses it is the half-life.",
        )
    };
    let cad_window = match as_of {
        Some(d) => format!("all history thru {d}"),
        None => "all history".to_string(),
    };
    let cad_sub = format!(
        "When commits land, by weekday and hour ({}, {cad_window}). \
         The grid is the rhythm (darker = busier); ▴/▾ beside a weekday or above an hour \
         marks where it's heating/cooling lately (warm = heating, cool = cooling). \
         Peak {} {:02}:00. weekends {:.0}%→{:.0}% · nights {:.0}%→{:.0}% (all-time → lately).",
        esc(&heat.tz),
        DAYS[heat.peak_day],
        heat.peak_hour,
        heat.weekend_all * 100.0,
        heat.weekend_lately * 100.0,
        heat.night_all * 100.0,
        heat.night_lately * 100.0,
    );
    // A loud, unmissable flag that the whole page is a point-in-time snapshot —
    // the report is shared, where reading a rewound view as "today" is costly.
    let asof_badge = match as_of {
        Some(d) => format!("<div class=asof>snapshot · as of {}</div>", esc(d)),
        None => String::new(),
    };
    // Coverage honesty, same caveat as the terminal header chip.
    let prov_badge = if c.is_provisional() {
        format!(
            "<div class=asof>provisional · {}wk of history</div>",
            c.coverage_weeks
        )
    } else {
        String::new()
    };

    let html = format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8>\
<meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>Terminal Velocity · {branch}</title><style>{css}</style></head>\
<body><main class=wrap>\
<header><div class=logo>v∞</div><h1>terminal velocity</h1><div class=meta>{branch} · {window_lbl}</div>{asof_badge}{prov_badge}</header>\
<section class=section><h2>{surv_h2}</h2>\
<p class=sub>{surv_sub}</p><div class=panel>{survival}</div>{surv_prose}</section>\
<section class=grid>{cards}</section>\
<section class=section><h2>cadence — when commits land</h2>\
<p class=sub>{cad_sub}</p><div class=panel>{cadence}</div></section>\
<section class=section><h2>thrash — in-place rewrite</h2>\
<p class=sub>{thr_sub}</p><div class=panel>{thrash}</div></section>\
<section class=section><h2>hotspots — refactor targets</h2>\
<p class=sub>{hot_sub}</p><div class=panel>{hot}</div></section>\
<footer>{footer}<div class=safety>Self-relative: each metric is judged against this \
repo's own recent baseline (recency-weighted), not external benchmarks.</div>\
<div class=builtby>built by <a href=\"https://github.com/onebit0fme\" rel=noopener>@onebit0fme</a></div>\
</footer></main></body></html>",
        branch = esc(&c.branch),
        window_lbl = esc(&c.window),
        asof_badge = asof_badge,
        prov_badge = prov_badge,
        // The drill-in hint ("run `tv thrash`…") is nonsense on the web — the
        // sections are right here. Keep only the net-churn line.
        footer = esc(c.footer.split(" · run").next().unwrap_or(&c.footer)),
        thr_sub = esc(&thr_sub),
        hot_sub = esc(&hot_sub),
        surv_h2 = esc(surv_h2),
        surv_sub = esc(surv_sub),
        surv_prose = report_survival_prose(),
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

/// A card's decision tree as the report's progressive-disclosure body: the metric's
/// `TREE_BODY` section (header prefix stripped), the fired branch bold, plus the
/// intent breakdown when present (thrash). Same source the terminal `--explain` uses.
fn report_card_explain(card: &Card) -> String {
    let Some(block) = tree_block(&card.key.to_uppercase()) else {
        return String::new();
    };
    let mut out = String::new();
    for (i, line) in block.lines().enumerate() {
        let text = if i == 0 {
            line.split_once(" · ").map_or(line, |(_, def)| def)
        } else {
            line
        };
        let trimmed = line.trim_start();
        let is_branch = trimmed.starts_with("├─") || trimmed.starts_with("└─");
        if is_branch && line.contains(card.state.as_str()) {
            out.push_str(&format!("<b>{}</b>\n", esc(text)));
        } else {
            out.push_str(&esc(text));
            out.push('\n');
        }
    }
    if let Some(d) = &card.detail {
        out.push_str(&format!("→ {}\n", esc(d)));
    }
    format!("<pre class=tree>{out}</pre>")
}

/// The survival story (the prose model) plus the Kaplan-Meier formula tucked behind
/// a disclosure at the bottom of the survival section — the web home for the story
/// the terminal `--explain` tells.
fn report_survival_prose() -> String {
    let story: String = SURVIVAL_STORY
        .iter()
        .map(|p| format!("<p>{}</p>", esc(p)))
        .collect();
    // Built per-line (not a `\`-continued literal — that strips the first line's
    // leading spaces and breaks the alignment of the stacked formula).
    let formula = [
        "           ⎛      dᵢ  ⎞",
        "  S(t) = ∏ ⎜ 1 − ──── ⎟",
        "       tᵢ≤t⎝      nᵢ  ⎠",
        "",
        "  dᵢ  deaths at age tᵢ    lines excised, not rewritten",
        "  nᵢ  still at risk        alive and not yet censored",
        "  t   age                  in commits, or days",
        "  →   feeds thrash weight w = S(age) · excision weight",
    ]
    .join("\n");
    // Recency is the board-wide lens, not Kaplan-Meier math — so it's its own caption,
    // not buried in the formula disclosure (mirrors the terminal: lens up top).
    let lens = format!(
        "<p class=lens>recency · every figure leans on recent weeks \
         (a commit's pull halves every ~{:.0}d): {}</p>",
        crate::metrics::RECENCY_HALFLIFE_DAYS,
        esc(&crate::metrics::recency_anchors())
    );
    format!(
        "{lens}<div class=story>{story}</div>\
         <details class=formula><summary>the math — Kaplan-Meier product-limit</summary>\
         <pre>{}</pre></details>",
        esc(&formula)
    )
}

/// The cadence punchcard as one CSS grid: the all-time *rhythm* (cell opacity = commit
/// count, busiest cell outlined), with the *shift* — which weekdays and hours are heating
/// or cooling lately — marked on the grid's own axes: a ▴/▾ above an hour column and
/// beside a weekday label (warm = heating, cool = cooling). Cells and markers carry hover
/// titles. No second grid — the row/column markers annotate the rhythm itself.
fn report_heatmap(h: &Heatmap) -> String {
    if h.total == 0 {
        return "<p class=empty>(no commits)</p>".to_string();
    }
    // (glyph, CSS class) for a margin's shift; both derive from the shared `shift_glyph`
    // bucketing so the report can't grade a slot differently than the terminal. "" = steady.
    let mark = |z: f64| -> (&'static str, &'static str) {
        match shift_glyph(z) {
            Some(g) => (g, if z > 0.0 { "heat" } else { "cool" }),
            None => ("", ""),
        }
    };
    let shift_title = |z: f64| -> String {
        match shift_glyph(z) {
            None => "steady".to_string(),
            Some(_) => {
                let dir = if z > 0.0 { "heating" } else { "cooling" };
                format!("{dir} {:.1}σ vs usual", z.abs())
            }
        }
    };

    let mut g = String::from("<div class=hgrid>");
    // Header row 1: hour-shift markers, over the columns. Row 2: hour ticks (every 3h).
    g.push_str("<span class=hcorner></span>");
    for hh in 0..24 {
        let (ch, cls) = mark(h.hour_shift[hh]);
        g.push_str(&format!(
            "<span class=\"hmark {cls}\" title=\"{hh:02}:00 · {}\">{ch}</span>",
            shift_title(h.hour_shift[hh])
        ));
    }
    g.push_str("<span class=hcorner></span>");
    for hh in 0..24 {
        let lbl = if hh % 3 == 0 {
            hh.to_string()
        } else {
            String::new()
        };
        g.push_str(&format!("<span class=hhour>{lbl}</span>"));
    }
    // Day rows: label tagged with its weekday-shift marker; cells shaded by raw count.
    for (d, row) in h.counts.iter().enumerate() {
        let (ch, cls) = mark(h.day_shift[d]);
        g.push_str(&format!(
            "<span class=\"hday {cls}\" title=\"{} · {}\">{ch}{}</span>",
            DAYS[d],
            shift_title(h.day_shift[d]),
            DAYS[d]
        ));
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
    let scale = tree
        .children
        .values()
        .map(|c| c.thrash)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let floor = thrash_floor(tree);
    let mut out = String::from("<div class=ttree>");
    report_branch(tree, scale, floor, 0, recent, &mut out);
    out.push_str("</div>");
    out
}

/// Folders with a kept child become collapsible `<details>` (top level open, deeper
/// folded); leaves are plain rows. Same global vital-few floor as the terminal.
fn report_branch(
    node: &TreeNode,
    scale: f64,
    floor: f64,
    depth: usize,
    recent: bool,
    out: &mut String,
) {
    for (name, child) in kept_children(node, floor) {
        let pct = child.thrash_pct();
        let t = tone_class(thr_tone(pct));
        let w = (child.thrash / scale * 100.0).clamp(2.0, 100.0);
        let row = format!(
            "<span class=\"tbar {t}\"><i style=\"width:{w:.0}%\"></i></span>\
             <span class=tname style=\"padding-left:{pad:.2}rem\"><span class=tw></span>{name}</span>\
             {traj}<span class=tnum>{thr:.0}</span><span class=tpct>{pct:.0}%</span>",
            pad = depth as f64 * 1.0,
            name = esc(name),
            traj = report_traj(child, recent),
            thr = child.thrash,
        );
        if kept_children(child, floor).is_empty() {
            out.push_str(&format!("<div class=\"trow leaf\">{row}</div>"));
        } else {
            let open = if depth == 0 { " open" } else { "" };
            out.push_str(&format!(
                "<details class=tnode{open}><summary class=trow>{row}</summary>"
            ));
            report_branch(child, scale, floor, depth + 1, recent, out);
            out.push_str("</details>");
        }
    }
}

/// Trajectory glyph for a folder (last-7d vs its recent pace), only when windowed.
fn report_traj(node: &TreeNode, recent: bool) -> String {
    if !recent {
        return String::new();
    }
    let (cls, ch) = match trend(node) {
        Trend::Heating => ("up", "↑"),
        Trend::Cooling => ("down", "↓"),
        Trend::Steady => ("flat", "→"),
    };
    format!("<span class=\"traj {cls}\">{ch}</span>")
}

/// The hotspots list as HTML rows — sqrt-scaled bars, repo tag when aggregating.
fn report_hotspots(rows: &[Hotspot], multi: bool) -> String {
    if rows.is_empty() {
        return "<p class=empty>(no files)</p>".to_string();
    }
    // Same vital-few cut as the terminal — the 80% of heat, no magic top-N.
    let cut = pareto_count(&rows.iter().map(|r| r.score).collect::<Vec<_>>(), VITAL_FEW).max(1);
    let max = rows[0].score.max(1.0); // sorted desc
    let mut out = String::new();
    for r in &rows[..cut] {
        let w = ((r.score / max).sqrt() * 100.0).clamp(2.0, 100.0);
        let meta = if multi {
            format!(
                "{}× · cx {} · rcy {:.2} · {}",
                r.freq,
                r.complexity,
                r.recency,
                esc(&r.repo)
            )
        } else {
            format!("{}× · cx {} · rcy {:.2}", r.freq, r.complexity, r.recency)
        };
        out.push_str(&format!(
            "<div class=hrow>\
               <span class=hbar><i style=\"width:{w:.0}%\"></i></span>\
               <span class=hfile>{file}</span><span class=hmeta>{meta}</span></div>",
            file = esc(&r.file),
        ));
    }
    if rows.len() > cut {
        out.push_str(&format!(
            "<div class=\"hrow more\">+{} more below the cut</div>",
            rows.len() - cut
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
