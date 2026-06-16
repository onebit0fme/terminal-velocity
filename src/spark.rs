//! Tiny presentation helpers: sparklines, plain and recency-weighted medians, and the
//! Wilson interval. Each metric is judged against this repo's own recent baseline
//! (recency-weighted), never an external benchmark.

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

/// Wilson score interval (95%) for a proportion `p` backed by effective sample size `n`.
/// Honest where the naive (Wald) interval lies: it stays inside [0,1] and stays *wide*
/// (never zero-width) at p=0/1 and small n — so thin data visibly abstains instead of
/// claiming false certainty. Returns `(lo, hi)`; `n <= 0` → maximally uncertain `(0,1)`.
pub fn wilson(p: f64, n: f64) -> (f64, f64) {
    const Z: f64 = 1.96;
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let z2 = Z * Z;
    let center = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
    let half = (Z / (1.0 + z2 / n)) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((center - half).max(0.0), (center + half).min(1.0))
}

/// Median of `(value, weight)` pairs — the value where cumulative weight first reaches
/// half the total. With recency weights it's a "typical, recent-leaning" value that's
/// still robust to the odd spike the way a plain median is. Empty / zero-weight → 0.0.
pub fn weighted_median(pairs: &[(f64, f64)]) -> f64 {
    let total: f64 = pairs.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut v = pairs.to_vec();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut acc = 0.0;
    for (val, w) in &v {
        acc += w;
        if acc >= total / 2.0 {
            return *val;
        }
    }
    v.last().map(|(val, _)| *val).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_median_matches_plain_median_at_equal_weights() {
        let eq: Vec<(f64, f64)> = [1.0, 2.0, 9.0].iter().map(|&v| (v, 1.0)).collect();
        assert_eq!(weighted_median(&eq), 2.0);
    }

    #[test]
    fn weighted_median_leans_toward_heavier_weights() {
        // heavy weight on the small value pulls the typical down off the plain median
        let pairs = [(1.0, 9.0), (50.0, 1.0), (51.0, 1.0)];
        assert_eq!(weighted_median(&pairs), 1.0);
        assert_eq!(weighted_median(&[]), 0.0);
    }

    #[test]
    fn wilson_stays_in_unit_interval_and_wide_at_extremes() {
        let (lo, hi) = wilson(0.0, 10.0);
        assert!(lo >= 0.0 && hi <= 1.0);
        assert!(hi > 0.2, "0/10 must be honestly wide, got hi={hi}");
        let (lo, hi) = wilson(1.0, 10.0);
        assert!((0.0..1.0).contains(&lo) && hi <= 1.0);
    }

    #[test]
    fn wilson_narrows_as_n_grows() {
        let (lo1, hi1) = wilson(0.5, 10.0);
        let (lo2, hi2) = wilson(0.5, 1000.0);
        assert!((hi2 - lo2) < (hi1 - lo1));
    }

    #[test]
    fn wilson_zero_n_is_maximally_uncertain() {
        assert_eq!(wilson(0.5, 0.0), (0.0, 1.0));
    }
}
