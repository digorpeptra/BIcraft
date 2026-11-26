//! Anomaly detection on a line chart's series (the Analytics pane's "Find anomalies",
//! `objects.anomalyDetection`): BIcraft's own estimate of each point's expected value and
//! expected range, and whether the point falls outside it.
//!
//! Microsoft Learn describes what Power BI shows (the expected range, the points outside it
//! marked as anomalies, a sensitivity that makes the detection stricter or looser, at least four
//! data points) and names the algorithm family it uses (SR-CNN, Azure Anomaly Detector), whose
//! trained model is not public. BIcraft does not reproduce it: the expected value is the median of
//! the point's neighbours (a centred window of `2h + 1` points, shifted inside the series at its
//! ends), the spread is the median absolute deviation of the residuals (scaled by 1.4826), and
//! the range is `expected ± k · spread` with `k` going from 5 (sensitivity 0) to 1 (sensitivity
//! 100). Results are labelled as BIcraft's estimate wherever they are shown
//! (docs/formats/pbir.md "Chart specifics", docs/gaps.md G56).

/// Fewest data points Power BI detects anomalies on (Microsoft Learn).
pub const MIN_POINTS: usize = 4;
/// Sensitivity used when the visual sets none (BIcraft's choice, in percent).
pub const DEFAULT_SENSITIVITY: f64 = 70.0;
/// Most points examined (more are left without an estimate).
pub const MAX_POINTS: usize = 5000;

/// One point's estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Expected {
    pub expected: f64,
    pub low: f64,
    pub high: f64,
    pub anomaly: bool,
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = xs.len();
    if n == 0 {
        return 0.0;
    }
    let mid = n / 2;
    let hi = xs.get(mid).copied().unwrap_or(0.0);
    if n % 2 == 1 {
        hi
    } else {
        let lo = xs.get(mid.saturating_sub(1)).copied().unwrap_or(hi);
        (lo + hi) / 2.0
    }
}

/// The range's half-width in spreads for `sensitivity` (0–100, clamped).
pub fn threshold(sensitivity: f64) -> f64 {
    let s = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 100.0)
    } else {
        DEFAULT_SENSITIVITY
    };
    1.0 + 4.0 * (100.0 - s) / 100.0
}

/// The estimate of each value of `vals` (in order; `None` where a value is missing), or an empty
/// list when fewer than [`MIN_POINTS`] values are present.
pub fn detect(vals: &[Option<f64>], sensitivity: f64) -> Vec<Option<Expected>> {
    let vals = vals.get(..vals.len().min(MAX_POINTS)).unwrap_or(&[]);
    let pts: Vec<(usize, f64)> = vals
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.filter(|x| x.is_finite()).map(|x| (i, x.clamp(-1e15, 1e15))))
        .collect();
    let n = pts.len();
    if n < MIN_POINTS {
        return Vec::new();
    }
    let h = (n / 8).clamp(1, 6);
    let w = (2 * h + 1).min(n);
    let mut expected = Vec::with_capacity(n);
    let mut buf: Vec<f64> = Vec::with_capacity(w);
    for k in 0..n {
        let start = k.saturating_sub(h).min(n.saturating_sub(w));
        buf.clear();
        buf.extend(pts.iter().skip(start).take(w).map(|p| p.1));
        expected.push(median(&mut buf));
    }
    let mut dev: Vec<f64> = pts.iter().zip(&expected).map(|(p, e)| (p.1 - e).abs()).collect();
    let mean = dev.iter().sum::<f64>() / n as f64;
    let mut spread = 1.4826 * median(&mut dev);
    if spread <= 0.0 {
        spread = mean;
    }
    if spread <= 0.0 {
        let scale = pts.iter().map(|p| p.1.abs()).fold(1.0f64, f64::max);
        spread = scale * 1e-9;
    }
    let k = threshold(sensitivity);
    let mut out = vec![None; vals.len()];
    for ((i, y), e) in pts.iter().zip(&expected) {
        let (low, high) = (e - k * spread, e + k * spread);
        if let Some(slot) = out.get_mut(*i) {
            *slot = Some(Expected {
                expected: *e,
                low,
                high,
                anomaly: *y < low || *y > high,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spike_is_an_anomaly_and_the_rest_is_not() {
        let mut vals: Vec<Option<f64>> = (0..20).map(|i| Some(100.0 + (i % 3) as f64)).collect();
        vals[10] = Some(180.0);
        vals[4] = None;
        let out = detect(&vals, DEFAULT_SENSITIVITY);
        assert_eq!(out.len(), 20);
        assert!(out[4].is_none());
        let flagged: Vec<usize> = (0..20).filter(|i| out[*i].is_some_and(|e| e.anomaly)).collect();
        assert_eq!(flagged, vec![10]);
        let e = out[10].unwrap();
        assert!(e.expected < 110.0 && e.high < 180.0, "{e:?}");
    }

    #[test]
    fn sensitivity_widens_or_narrows_the_range() {
        let vals: Vec<Option<f64>> = [10.0, 12.0, 9.0, 14.0, 11.0, 18.0, 10.0, 13.0]
            .iter()
            .map(|x| Some(*x))
            .collect();
        let strict = detect(&vals, 100.0);
        let loose = detect(&vals, 0.0);
        let width = |o: &[Option<Expected>]| o[5].map(|e| e.high - e.low).unwrap();
        assert!(width(&strict) < width(&loose));
        let count = |o: &[Option<Expected>]| o.iter().flatten().filter(|e| e.anomaly).count();
        assert!(count(&strict) >= count(&loose));
        assert_eq!(threshold(f64::NAN), threshold(DEFAULT_SENSITIVITY));
    }

    #[test]
    fn too_few_points_or_hostile_values_give_nothing_or_finite_results() {
        assert!(detect(&[Some(1.0), Some(2.0), None, Some(3.0)], 70.0).is_empty());
        assert!(detect(&[], 70.0).is_empty());
        let flat = detect(&[Some(5.0); 6], 70.0);
        assert!(flat.iter().flatten().all(|e| !e.anomaly && e.low <= e.high));
        let wild = [
            Some(f64::MAX),
            Some(f64::NAN),
            Some(-1e300),
            Some(1.0),
            Some(f64::INFINITY),
            Some(2.0),
            Some(3.0),
        ];
        for e in detect(&wild, 200.0).into_iter().flatten() {
            assert!(e.expected.is_finite() && e.low.is_finite() && e.high.is_finite());
        }
    }
}
