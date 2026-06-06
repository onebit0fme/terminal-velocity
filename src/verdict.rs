//! The verdict composer — deterministic, rule-based, no LLM at runtime.
//!
//! Non-negotiable #1: a human-readable "build flow is X; watch Y; Z is fine"
//! above the charts. Composed from metric states, so it's fast, offline, and
//! reproducible. A learned model may *polish* this later via an external call,
//! but the composer is always the zero-dependency default.

pub struct Signals {
    pub coverage_weeks: usize,
    pub batch_state: String, // steady / rising / easing
    pub batch_from: f64,     // overall median churn
    pub batch_to: f64,       // recent-week median churn
    pub nights_recent: f64,  // % of recent commits at night
    pub nights_base: f64,    // % baseline (sustained)
    pub weekend_pct: f64,    // % of all commits on weekends
    pub thrash_pct: f64,     // S-weighted thrash as % of churn
    pub net: i64,
}

pub fn compose(s: &Signals) -> String {
    // Coverage honesty (non-negotiable #7): no confident trends on a young repo.
    if s.coverage_weeks < 3 {
        return format!(
            "BUILDING BASELINE ({} wk) — trends are provisional until ~3 weeks of history.",
            s.coverage_weeks
        );
    }

    let mut lead = String::from("BUILD FLOW: ");
    lead.push_str(match s.batch_state.as_str() {
        "rising" => "steady, batches creeping",
        "easing" => "steady, batches tightening",
        _ => "steady",
    });

    // Thrash is the headline risk signal.
    if s.thrash_pct < 8.0 {
        lead.push_str(". No thrash spiral");
    } else if s.thrash_pct < 15.0 {
        lead.push_str(&format!(". Thrash elevated ({:.0}%)", s.thrash_pct));
    } else {
        lead.push_str(&format!(". Thrash HIGH ({:.0}%)", s.thrash_pct));
    }

    let mut watch = Vec::new();
    if s.batch_state == "rising" {
        watch.push(format!(
            "batch median {:.0}→{:.0}, split smaller",
            s.batch_from, s.batch_to
        ));
    }
    if s.nights_recent > s.nights_base + 7.0 {
        watch.push(format!(
            "night work climbing {:.0}→{:.0}% — protect rest",
            s.nights_base, s.nights_recent
        ));
    } else if s.weekend_pct > 35.0 {
        watch.push(format!(
            "{:.0}% of commits land on weekends — protect recovery",
            s.weekend_pct
        ));
    } else if s.nights_base > 25.0 {
        watch.push(format!(
            "{:.0}% of commits at night — protect recovery",
            s.nights_base
        ));
    }
    if s.thrash_pct >= 15.0 {
        watch.push("thrash high — stabilize this area before adding".to_string());
    }

    let mut out = lead;
    out.push('.');
    if watch.is_empty() {
        out.push_str(if s.net >= 0 {
            " Nothing drifting — you're building, not spinning."
        } else {
            " Nothing drifting — net subtractive, you're consolidating."
        });
    } else {
        out.push_str("  Watch: ");
        out.push_str(&watch.join("; "));
        out.push('.');
    }
    out
}
