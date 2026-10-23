//! Value-at-λ traces of multi-column files: for a temperature series this is
//! the melting / association curve, y(λ) of every scan against its label.

use egui::Color32;

use crate::color::{self, ColorMode};
use crate::jasco::Dataset;
use crate::processing::{self, Baseline, Normalize};

/// Why a file cannot be traced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotTraceable {
    /// The file holds a single curve, so there is nothing to trace across.
    SingleCurve,
    /// Some column label is not a finite number, so it cannot be an x value.
    NonNumericLabels,
}

impl NotTraceable {
    /// A short hint shown in place of the trace.
    pub fn reason(self) -> &'static str {
        match self {
            Self::SingleCurve => "Needs a file with several columns, e.g. a temperature series.",
            Self::NonNumericLabels => {
                "Needs every column label to be a number, e.g. the scan temperature."
            }
        }
    }
}

/// The trace of `dataset` at x = `at`: one `[label value, y]` point per shown
/// curve, in column order.
///
/// `y` is the curve after `baseline` and `normalize` (the same values the
/// plot draws), linearly interpolated at `at`. A curve whose x range does not
/// contain `at`, or whose value there is not finite, gives no point.
///
/// `shown` holds one flag per curve, as the curve list does.
pub fn trace_points(
    dataset: &Dataset,
    shown: &[bool],
    at: f64,
    baseline: &Baseline,
    normalize: &Normalize,
) -> Result<Vec<[f64; 2]>, NotTraceable> {
    if !dataset.is_multi() {
        return Err(NotTraceable::SingleCurve);
    }
    let labels = color::label_values(dataset.curves.iter().map(|c| c.label.as_str()))
        .filter(|values| values.iter().all(|v| v.is_finite()))
        .ok_or(NotTraceable::NonNumericLabels)?;
    debug_assert_eq!(shown.len(), dataset.curves.len(), "one flag per curve");

    let points = dataset
        .curves
        .iter()
        .zip(labels)
        .zip(shown)
        .filter(|&(_, &shown)| shown)
        .filter_map(|((curve, label), _)| {
            let processed = processing::apply(curve, baseline, normalize);
            let (x, y): (Vec<f64>, Vec<f64>) = processed.iter().map(|p| (p[0], p[1])).unzip();
            processing::interp(&x, &y, at)
                .filter(|y| y.is_finite())
                .map(|y| [label, y])
        })
        .collect();
    Ok(points)
}

/// Colour of the `nth` trace: a single-colour file keeps its colour, a
/// gradient file (whose curves have no one colour) takes a palette colour.
pub fn trace_color(mode: ColorMode, nth: usize) -> Color32 {
    match mode {
        ColorMode::Single(c) => c,
        ColorMode::Gradient(_) => color::palette(nth),
    }
}

/// The x and y axis titles of a trace at `at`, for files whose x unit is
/// `x_unit` and whose y axis is titled `y_title`.
pub fn trace_axis_titles(x_unit: &str, y_title: &str, at: f64) -> (String, String) {
    let at = format_x(at);
    let y = if x_unit == "NANOMETERS" {
        format!("{y_title} at {at} nm")
    } else {
        format!("{y_title} at x = {at}")
    };
    ("Column label".to_owned(), y)
}

/// `x` with at most two decimals and no trailing zeros: 395, 395.5, 395.13.
fn format_x(x: f64) -> String {
    let s = format!("{x:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Colormap;
    use crate::fixtures::{self, multi, single};
    use crate::jasco::{self, Curve};

    fn multi_dataset() -> Dataset {
        jasco::parse(&fixtures::read(multi::FILE), "multi").unwrap()
    }

    /// The row of the multi fixture whose x is `x`.
    fn row(ds: &Dataset, x: f64) -> usize {
        ds.curves[0]
            .x
            .iter()
            .position(|&v| v == x)
            .unwrap_or_else(|| panic!("no row at {x}"))
    }

    fn fixture_label_values() -> Vec<f64> {
        multi::LABELS.iter().map(|l| l.parse().unwrap()).collect()
    }

    fn all_shown(ds: &Dataset) -> Vec<bool> {
        vec![true; ds.curves.len()]
    }

    fn trace(ds: &Dataset, at: f64) -> Result<Vec<[f64; 2]>, NotTraceable> {
        trace_points(ds, &all_shown(ds), at, &Baseline::None, &Normalize::None)
    }

    fn curve(label: &str, y: [f64; 3]) -> Curve {
        Curve {
            label: label.into(),
            x: vec![400.0, 395.0, 390.0],
            y: y.to_vec(),
        }
    }

    fn dataset(curves: Vec<Curve>) -> Dataset {
        Dataset {
            name: "made".into(),
            x_unit: "NANOMETERS".into(),
            y_unit: "ABSORBANCE".into(),
            curves,
            ..Default::default()
        }
    }

    #[test]
    fn grid_point_gives_the_file_values_against_the_labels() {
        let ds = multi_dataset();
        let i = row(&ds, 395.0);
        let points = trace(&ds, 395.0).unwrap();

        let xs: Vec<f64> = points.iter().map(|p| p[0]).collect();
        assert_eq!(xs, [85.0, 70.02, 55.0, 39.98, 25.01, 10.0]);
        assert_eq!(xs, fixture_label_values());
        for (k, p) in points.iter().enumerate() {
            assert_eq!(p[1], ds.curves[k].y[i]);
            // ... which is the generator's value, to the six digits written.
            let want = multi::y(k, i);
            assert!((p[1] - want).abs() <= want.abs() * 1e-5, "{p:?} vs {want}");
        }
    }

    #[test]
    fn between_grid_points_gives_the_midpoint() {
        let ds = multi_dataset();
        let (a, b) = (row(&ds, 396.0), row(&ds, 395.0));
        let points = trace(&ds, 395.5).unwrap();
        assert_eq!(points.len(), multi::LABELS.len());
        for (k, p) in points.iter().enumerate() {
            let c = &ds.curves[k];
            let want = (c.y[a] + c.y[b]) / 2.0;
            assert!((p[1] - want).abs() < 1e-12, "{} vs {want}", p[1]);
        }
    }

    #[test]
    fn outside_the_data_gives_no_points() {
        let ds = multi_dataset();
        let first = ds.curves[0].x[0];
        assert!(first < 500.0, "fixture now reaches 500 nm ({first})");
        assert_eq!(trace(&ds, 500.0), Ok(vec![]));
    }

    #[test]
    fn uses_the_processed_curves() {
        let ds = multi_dataset();
        let baseline = Baseline::RangeMean {
            lo: 450.0,
            hi: 460.0,
        };
        let normalize = Normalize::Max;
        let points = trace_points(&ds, &all_shown(&ds), 395.0, &baseline, &normalize).unwrap();
        let raw = trace(&ds, 395.0).unwrap();

        assert_eq!(points.len(), ds.curves.len());
        for (k, p) in points.iter().enumerate() {
            let processed = processing::apply(&ds.curves[k], &baseline, &normalize);
            let (x, y): (Vec<f64>, Vec<f64>) = processed.iter().map(|p| (p[0], p[1])).unzip();
            assert_eq!(p[1], processing::interp(&x, &y, 395.0).unwrap());
            assert_ne!(p[1], raw[k][1], "column {k} was not processed");
        }
    }

    #[test]
    fn hidden_curves_are_skipped() {
        let ds = multi_dataset();
        let mut shown = all_shown(&ds);
        shown[1] = false;
        let points = trace_points(&ds, &shown, 395.0, &Baseline::None, &Normalize::None).unwrap();
        let xs: Vec<f64> = points.iter().map(|p| p[0]).collect();
        assert_eq!(xs, [85.0, 55.0, 39.98, 25.01, 10.0]);
    }

    #[test]
    fn single_curve_files_are_not_traceable() {
        let ds = jasco::parse(&fixtures::read(single::FILE), "single").unwrap();
        assert_eq!(trace(&ds, 395.0), Err(NotTraceable::SingleCurve));
    }

    #[test]
    fn non_numeric_labels_are_not_traceable() {
        for bad in ["warm", "nan", "inf", ""] {
            let ds = dataset(vec![curve("20", [1.0; 3]), curve(bad, [2.0; 3])]);
            assert_eq!(
                trace(&ds, 395.0),
                Err(NotTraceable::NonNumericLabels),
                "label {bad:?}"
            );
        }
    }

    #[test]
    fn a_nan_value_drops_only_that_point() {
        let ds = dataset(vec![
            curve("20", [1.0, 2.0, 3.0]),
            curve("40", [1.0, f64::NAN, 3.0]),
            curve("60", [1.0, 4.0, 3.0]),
        ]);
        assert_eq!(trace(&ds, 395.0), Ok(vec![[20.0, 2.0], [60.0, 4.0]]));
    }

    #[test]
    fn trace_color_keeps_a_single_colour_and_gives_gradients_a_palette_colour() {
        let red = Color32::from_rgb(200, 10, 10);
        assert_eq!(trace_color(ColorMode::Single(red), 3), red);
        let gradient = ColorMode::Gradient(Colormap::Coolwarm);
        assert_eq!(trace_color(gradient, 0), color::palette(0));
        assert_eq!(trace_color(gradient, 2), color::palette(2));
    }

    #[test]
    fn axis_titles_name_the_wavelength() {
        assert_eq!(
            trace_axis_titles("NANOMETERS", "Absorbance", 395.0),
            ("Column label".to_owned(), "Absorbance at 395 nm".to_owned())
        );
        assert_eq!(
            trace_axis_titles("NANOMETERS", "Normalized", 395.5).1,
            "Normalized at 395.5 nm"
        );
        assert_eq!(
            trace_axis_titles("NANOMETERS", "Absorbance", 395.126).1,
            "Absorbance at 395.13 nm"
        );
        assert_eq!(
            trace_axis_titles("SEC", "Absorbance", 30.0),
            ("Column label".to_owned(), "Absorbance at x = 30".to_owned())
        );
        // Rounding a small negative value to two decimals does not print "-0".
        assert_eq!(trace_axis_titles("SEC", "y", -0.001).1, "y at x = 0");
    }

    #[test]
    fn reasons_are_distinct_hints() {
        let single = NotTraceable::SingleCurve.reason();
        let labels = NotTraceable::NonNumericLabels.reason();
        assert!(!single.is_empty() && !labels.is_empty());
        assert_ne!(single, labels);
    }
}
