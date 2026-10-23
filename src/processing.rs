//! Baseline correction and normalisation, applied to every curve in that order.

use serde::{Deserialize, Serialize};

use crate::jasco::Curve;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Baseline {
    None,
    /// Subtract the mean of the points whose x lies in `lo..=hi`.
    RangeMean {
        lo: f64,
        hi: f64,
    },
    /// Subtract the straight line through the curve at `x1` and `x2`.
    TwoPoint {
        x1: f64,
        x2: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Normalize {
    None,
    /// Divide by the maximum value, so the peak becomes 1.
    Max,
    /// Divide by the value at this x, so that point becomes 1.
    AtX(f64),
}

/// Values this close to zero are not used as a divisor.
const MIN_DIVISOR: f64 = 1e-12;

/// Linear interpolation of `y` at `at`. `x` may be ascending or descending.
pub fn interp(x: &[f64], y: &[f64], at: f64) -> Option<f64> {
    x.windows(2).zip(y.windows(2)).find_map(|(xs, ys)| {
        let (lo, hi) = if xs[0] <= xs[1] {
            (xs[0], xs[1])
        } else {
            (xs[1], xs[0])
        };
        if !(lo..=hi).contains(&at) {
            return None;
        }
        if xs[1] == xs[0] {
            return Some(ys[0]);
        }
        let t = (at - xs[0]) / (xs[1] - xs[0]);
        Some(ys[0] + t * (ys[1] - ys[0]))
    })
}

fn baseline_values(curve: &Curve, baseline: &Baseline) -> Option<Vec<f64>> {
    match *baseline {
        Baseline::None => None,
        Baseline::RangeMean { lo, hi } => {
            let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
            let in_range: Vec<f64> = curve
                .x
                .iter()
                .zip(&curve.y)
                .filter(|(x, _)| (lo..=hi).contains(*x))
                .map(|(_, y)| *y)
                .collect();
            let offset = if in_range.is_empty() {
                interp(&curve.x, &curve.y, (lo + hi) / 2.0)?
            } else {
                in_range.iter().sum::<f64>() / in_range.len() as f64
            };
            Some(vec![offset; curve.x.len()])
        }
        Baseline::TwoPoint { x1, x2 } => {
            let y1 = interp(&curve.x, &curve.y, x1)?;
            let y2 = interp(&curve.x, &curve.y, x2)?;
            if x1 == x2 {
                return Some(vec![y1; curve.x.len()]);
            }
            let slope = (y2 - y1) / (x2 - x1);
            Some(curve.x.iter().map(|x| y1 + slope * (x - x1)).collect())
        }
    }
}

/// Returns the processed curve as plot points.
///
/// A baseline or normalisation point that falls outside the curve's x range
/// leaves that step unapplied rather than hiding the curve. Likewise, `Max`
/// is not applied when the curve has no positive maximum.
pub fn apply(curve: &Curve, baseline: &Baseline, normalize: &Normalize) -> Vec<[f64; 2]> {
    let mut y = curve.y.clone();
    if let Some(base) = baseline_values(curve, baseline) {
        for (v, b) in y.iter_mut().zip(base) {
            *v -= b;
        }
    }

    let divisor = match *normalize {
        Normalize::None => None,
        // Only a positive maximum can be scaled to 1; dividing by a negative
        // one would flip the curve, so leave it unnormalised instead.
        Normalize::Max => y
            .iter()
            .copied()
            .reduce(f64::max)
            .filter(|d| *d > MIN_DIVISOR),
        Normalize::AtX(at) => interp(&curve.x, &y, at).filter(|d| d.abs() > MIN_DIVISOR),
    };
    if let Some(d) = divisor {
        for v in &mut y {
            *v /= d;
        }
    }

    curve.x.iter().zip(y).map(|(x, y)| [*x, y]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Descending x, as in the JASCO wavelength scans: y = 0.01 * x + 1.
    fn line_curve() -> Curve {
        let x: Vec<f64> = (0..=10).rev().map(|i| i as f64 * 10.0).collect();
        let y = x.iter().map(|x| 0.01 * x + 1.0).collect();
        Curve {
            label: "t".into(),
            x,
            y,
        }
    }

    fn ys(points: &[[f64; 2]]) -> Vec<f64> {
        points.iter().map(|p| p[1]).collect()
    }

    #[test]
    fn interp_handles_descending_x() {
        let c = line_curve();
        assert!((interp(&c.x, &c.y, 45.0).unwrap() - 1.45).abs() < 1e-12);
        assert!(interp(&c.x, &c.y, 150.0).is_none());
    }

    #[test]
    fn range_mean_subtracts_the_average() {
        let c = line_curve();
        // Points at 80, 90, 100 → mean y = 1.9.
        let p = apply(
            &c,
            &Baseline::RangeMean {
                lo: 100.0,
                hi: 80.0,
            },
            &Normalize::None,
        );
        assert!((p[0][1] - (2.0 - 1.9)).abs() < 1e-12);
        assert!((p[10][1] - (1.0 - 1.9)).abs() < 1e-12);
    }

    /// Asserts every y equals the original y minus `offset`, with no NaN.
    fn assert_shifted_by(p: &[[f64; 2]], c: &Curve, offset: f64) {
        for (got, orig) in ys(p).iter().zip(&c.y) {
            assert!(
                (got - (orig - offset)).abs() < 1e-12,
                "got {got}, want {}",
                orig - offset
            );
        }
    }

    #[test]
    fn range_mean_without_points_subtracts_midpoint_value() {
        let c = line_curve();
        // No grid point (x step 10) lies at 45, so the baseline falls back to
        // the interpolated y(45) = 1.45 instead of averaging nothing.
        let p = apply(
            &c,
            &Baseline::RangeMean { lo: 45.0, hi: 45.0 },
            &Normalize::None,
        );
        assert_shifted_by(&p, &c, 1.45);
    }

    #[test]
    fn range_mean_reversed_range_without_points_uses_same_midpoint() {
        let c = line_curve();
        // lo > hi is reordered to 42..=48: no grid point inside, midpoint 45.
        let p = apply(
            &c,
            &Baseline::RangeMean { lo: 48.0, hi: 42.0 },
            &Normalize::None,
        );
        assert_shifted_by(&p, &c, 1.45);
    }

    #[test]
    fn range_mean_outside_data_leaves_curve_unchanged() {
        let c = line_curve();
        let p = apply(
            &c,
            &Baseline::RangeMean {
                lo: 200.0,
                hi: 300.0,
            },
            &Normalize::None,
        );
        assert_eq!(ys(&p), c.y);
    }

    #[test]
    fn two_point_flattens_a_sloped_line() {
        let c = line_curve();
        let p = apply(
            &c,
            &Baseline::TwoPoint { x1: 15.0, x2: 85.0 },
            &Normalize::None,
        );
        assert!(ys(&p).iter().all(|y| y.abs() < 1e-12));
    }

    #[test]
    fn normalize_max_scales_peak_to_one() {
        let c = line_curve();
        let p = apply(&c, &Baseline::None, &Normalize::Max);
        assert!((ys(&p).iter().copied().reduce(f64::max).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn normalize_max_leaves_all_negative_curve_unchanged() {
        // No positive maximum exists to scale to 1; dividing by -0.1 would
        // flip the curve and blow it up to [1, 5, 10].
        let c = Curve {
            label: "t".into(),
            x: vec![3.0, 2.0, 1.0],
            y: vec![-0.1, -0.5, -1.0],
        };
        let p = apply(&c, &Baseline::None, &Normalize::Max);
        assert_eq!(ys(&p), c.y);
    }

    #[test]
    fn normalize_at_x_runs_after_baseline() {
        let c = line_curve();
        let p = apply(
            &c,
            &Baseline::RangeMean { lo: 0.0, hi: 0.0 },
            &Normalize::AtX(50.0),
        );
        // After subtracting y(0) = 1, y = 0.01x; dividing by y(50) = 0.5 gives 0.02x.
        assert!((p[0][1] - 2.0).abs() < 1e-12);
        assert!((p[5][1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn out_of_range_settings_leave_curve_unchanged() {
        let c = line_curve();
        let p = apply(
            &c,
            &Baseline::TwoPoint {
                x1: 500.0,
                x2: 600.0,
            },
            &Normalize::AtX(999.0),
        );
        assert_eq!(ys(&p), c.y);
    }
}
