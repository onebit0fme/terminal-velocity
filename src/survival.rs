//! Kaplan-Meier survival over line lifetimes — the self-calibrating yardstick.
//!
//! No magic N-day window: the repo's own line-lifetime distribution is the axis.
//! Ported from the Python spike. Currently exercised by tests; it gets wired to
//! real data once the blame-at-death collection lands (see `git::collect_deaths`).
//!
//! Event vs censored matters: an in-place *rewrite* is a line that survived
//! (modified), so it is censored, not a death. Only true *excision* is an event.
//! Treating rewrites as deaths shortens the curve (validated in the spike).

fn cmp_f64(a: &f64, b: &f64) -> std::cmp::Ordering {
    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Returns (times, S) as a right-continuous step function.
/// `events` = true-death ages; `censored` = alive-at-HEAD or modified ages.
pub fn km_survival(events: &[f64], censored: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut obs: Vec<(f64, bool)> = Vec::with_capacity(events.len() + censored.len());
    for &a in events {
        obs.push((a, true));
    }
    for &a in censored {
        obs.push((a, false));
    }
    obs.sort_by(|x, y| cmp_f64(&x.0, &y.0));

    let mut times = Vec::new();
    let mut surv = Vec::new();
    let mut s = 1.0_f64;
    let mut n = obs.len() as f64;
    let mut i = 0;
    while i < obs.len() {
        let t = obs[i].0;
        let mut j = i;
        let mut deaths = 0.0_f64;
        while j < obs.len() && obs[j].0 == t {
            if obs[j].1 {
                deaths += 1.0;
            }
            j += 1;
        }
        if deaths > 0.0 {
            s *= 1.0 - deaths / n;
            times.push(t);
            surv.push(s);
        }
        n -= (j - i) as f64;
        i = j;
    }
    (times, surv)
}

/// S(age): fraction expected to outlive `age`. 1.0 below the first death time.
pub fn survival_at(times: &[f64], surv: &[f64], age: f64) -> f64 {
    let mut result = 1.0;
    for (t, s) in times.iter().zip(surv.iter()) {
        if *t <= age {
            result = *s;
        } else {
            break;
        }
    }
    result
}

/// First age where S <= 0.5, or None if the median is never reached
/// (>50% of lines never die — common in young/durable repos).
pub fn half_life(times: &[f64], surv: &[f64]) -> Option<f64> {
    for (t, s) in times.iter().zip(surv.iter()) {
        if *s <= 0.5 {
            return Some(*t);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_life_uncensored() {
        // 2 deaths -> S = .5 (exact) at age 1, then 0 at age 2 -> median = 1
        let (t, s) = km_survival(&[1.0, 2.0], &[]);
        assert_eq!(half_life(&t, &s), Some(1.0));
    }

    #[test]
    fn censoring_lengthens_or_prevents_median() {
        // one early death, many censored survivors -> median not reached
        let (t, s) = km_survival(&[1.0], &[10.0, 10.0, 10.0, 10.0]);
        assert_eq!(half_life(&t, &s), None);
    }

    #[test]
    fn survival_lookup_is_step() {
        let (t, s) = km_survival(&[1.0, 2.0, 3.0, 4.0], &[]);
        assert_eq!(survival_at(&t, &s, 0.0), 1.0);
        assert!((survival_at(&t, &s, 2.0) - 0.5).abs() < 1e-9);
    }
}
