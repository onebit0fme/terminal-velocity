//! Tiny presentation helpers: sparklines, percentile-against-your-own-history,
//! median. Every cockpit number is a trend against a trailing baseline, and
//! every threshold is a percentile against *this repo's* distribution — never
//! an external benchmark.

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    values
        .iter()
        .map(|v| {
            let pos = ((v - min) / span) * (BARS.len() as f64 - 1.0);
            let idx = (pos.round() as usize).min(BARS.len() - 1);
            BARS[idx]
        })
        .collect()
}

/// Percentile rank of `value` within `history` (0..100). "p78 for you".
pub fn percentile_rank(value: f64, history: &[f64]) -> f64 {
    if history.is_empty() {
        return f64::NAN;
    }
    let below = history.iter().filter(|&&h| h < value).count() as f64;
    below / history.len() as f64 * 100.0
}

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}
