//! Writing the displayed curves to an image file (PNG or SVG) with ruviz.
//!
//! [`write_figure`] takes plain data and a path, so it does not depend on the
//! UI and the files it writes are checked by unit tests.
//!
//! ruviz decides nothing from the file name ([`ruviz::prelude::Plot::save`]
//! always writes PNG, `export_svg` always SVG), so the format is chosen here
//! from the extension, and an unknown extension is refused before anything
//! is written.

use std::path::Path;

use egui::Color32;
use ruviz::prelude::{Color, Plot};

/// One curve as it is to be drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureCurve {
    /// Legend entry.
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub color: Color32,
}

/// Everything the figure shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub curves: Vec<FigureCurve>,
    pub x_title: String,
    pub y_title: String,
    /// Whether to draw a legend of the curve names.
    pub legend: bool,
}

/// File extensions [`write_figure`] accepts, lower case and without the dot.
pub const EXTENSIONS: &[&str] = &["png", "svg"];

/// Line width of every curve, the same as on screen.
const LINE_WIDTH: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Svg,
}

impl Format {
    /// The format named by the extension of `path`, ignoring case.
    fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }
}

/// Draws `figure` and writes it to `path`, as PNG or SVG by its extension.
///
/// Non-finite points are left out; a curve without any finite point is not
/// drawn. Fails, without writing, when the extension is neither `.png` nor
/// `.svg` or when no curve has a finite point. Missing parent directories are
/// created. On failure the error reads `{path}: {reason}`, like
/// [`crate::jasco::load`].
pub fn write_figure(path: &Path, figure: &Figure) -> Result<(), String> {
    let fail = |reason: String| format!("{}: {reason}", path.display());
    let format = Format::of(path).ok_or_else(|| {
        fail(format!(
            "unsupported figure format (supported: {})",
            EXTENSIONS
                .iter()
                .map(|e| format!(".{e}"))
                .collect::<Vec<_>>()
                .join(" / ")
        ))
    })?;

    let mut plot = Plot::new()
        .xlabel(figure.x_title.as_str())
        .ylabel(figure.y_title.as_str());
    let mut drawn = 0;
    for curve in &figure.curves {
        let (xs, ys): (Vec<f64>, Vec<f64>) = curve
            .points
            .iter()
            .filter(|[x, y]| x.is_finite() && y.is_finite())
            .map(|&[x, y]| (x, y))
            .unzip();
        if xs.is_empty() {
            continue;
        }
        let [r, g, b, a] = curve.color.to_srgba_unmultiplied();
        plot = plot
            .line(&xs, &ys)
            .label(curve.name.as_str())
            .color(Color::from_rgba(r, g, b, a))
            .line_width(LINE_WIDTH)
            .into();
        drawn += 1;
    }
    if drawn == 0 {
        return Err(fail("no curve with finite points to draw".into()));
    }
    if figure.legend {
        plot = plot.legend_best();
    }

    match format {
        Format::Png => plot.save(path),
        Format::Svg => plot.export_svg(path),
    }
    .map_err(|e| fail(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::fixtures;
    use crate::jasco;
    use crate::processing::{self, Baseline, Normalize};

    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

    /// A temporary directory removed when dropped (also when the test panics).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("uv_viewer-figure-{test}-{}", std::process::id()));
            // Clear a leftover from an earlier run that had the same PID.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Names of the entries directly in the directory, sorted.
        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .expect("read temp dir")
                .map(|e| {
                    e.expect("dir entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Best-effort cleanup: a leftover temp directory must not fail the test.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The multi column fixture, unprocessed, as figure curves.
    fn fixture_figure() -> Figure {
        let dataset =
            jasco::load(&fixtures::dir().join(fixtures::multi::FILE)).expect("load fixture");
        let curves = dataset
            .curves
            .iter()
            .enumerate()
            .map(|(k, c)| FigureCurve {
                name: format!("{} {}", dataset.name, c.label),
                points: processing::apply(c, &Baseline::None, &Normalize::None),
                color: Color32::from_rgb(40 * k as u8, 80, 200 - 30 * k as u8),
            })
            .collect();
        Figure {
            curves,
            x_title: dataset.x_unit.clone(),
            y_title: dataset.y_unit.clone(),
            legend: true,
        }
    }

    fn one_curve(name: &str, points: Vec<[f64; 2]>) -> Figure {
        Figure {
            curves: vec![FigureCurve {
                name: name.into(),
                points,
                color: Color32::RED,
            }],
            x_title: "Wavelength [nm]".into(),
            y_title: "Abs".into(),
            legend: true,
        }
    }

    #[test]
    fn writes_fixture_as_png() {
        let dir = TempDir::new("png");
        let path = dir.path("figure.png");
        write_figure(&path, &fixture_figure()).expect("write PNG");
        let bytes = std::fs::read(&path).expect("read PNG");
        assert!(bytes.starts_with(PNG_SIGNATURE), "not a PNG file");
    }

    #[test]
    fn writes_fixture_as_svg_with_legend_and_axis_titles() {
        let dir = TempDir::new("svg");
        let path = dir.path("figure.svg");
        let figure = fixture_figure();
        write_figure(&path, &figure).expect("write SVG");
        let svg = std::fs::read_to_string(&path).expect("read SVG");
        assert!(svg.contains("<svg"), "not an SVG file");
        for curve in &figure.curves {
            assert!(svg.contains(&curve.name), "legend lacks {:?}", curve.name);
        }
        assert!(
            svg.contains(&figure.x_title),
            "no x title {:?}",
            figure.x_title
        );
        assert!(
            svg.contains(&figure.y_title),
            "no y title {:?}",
            figure.y_title
        );
    }

    #[test]
    fn extension_is_matched_ignoring_case() {
        let dir = TempDir::new("upper");
        let figure = one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]);
        let png = dir.path("figure.PNG");
        let svg = dir.path("figure.Svg");
        write_figure(&png, &figure).expect("write .PNG");
        write_figure(&svg, &figure).expect("write .Svg");
        assert!(std::fs::read(&png).unwrap().starts_with(PNG_SIGNATURE));
        assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    }

    #[test]
    fn japanese_legend_name_is_kept_in_svg() {
        let dir = TempDir::new("japanese");
        let path = dir.path("figure.svg");
        let name = "合成試料その一 25℃";
        write_figure(&path, &one_curve(name, vec![[1.0, 1.0], [2.0, 3.0]])).expect("write SVG");
        let svg = std::fs::read_to_string(&path).expect("read SVG");
        assert!(svg.contains(name), "SVG lacks {name:?}");
    }

    #[test]
    fn unsupported_extension_is_refused_without_writing() {
        let dir = TempDir::new("txt");
        let figure = one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]);
        for name in ["figure.txt", "figure"] {
            let path = dir.path(name);
            let err = write_figure(&path, &figure).expect_err("must refuse");
            assert!(err.starts_with(&path.display().to_string()), "{err}");
            assert!(err.contains("unsupported figure format"), "{err}");
            assert!(err.contains(".png") && err.contains(".svg"), "{err}");
        }
        assert!(dir.entries().is_empty(), "wrote {:?}", dir.entries());
    }

    #[test]
    fn path_below_a_regular_file_is_an_error() {
        let dir = TempDir::new("below-file");
        let file = dir.path("file");
        std::fs::write(&file, b"x").expect("write file");
        let path = file.join("figure.png");
        let err = write_figure(&path, &one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]))
            .expect_err("must fail");
        assert!(err.starts_with(&path.display().to_string()), "{err}");
    }

    #[test]
    fn non_finite_points_are_left_out() {
        let dir = TempDir::new("nan");
        let figure = Figure {
            curves: vec![
                FigureCurve {
                    name: "with gaps".into(),
                    points: vec![
                        [1.0, 1.0],
                        [2.0, f64::NAN],
                        [f64::INFINITY, 2.0],
                        [3.0, 2.0],
                    ],
                    color: Color32::RED,
                },
                FigureCurve {
                    name: "all NaN".into(),
                    points: vec![[f64::NAN, f64::NAN]],
                    color: Color32::BLUE,
                },
            ],
            x_title: "x".into(),
            y_title: "y".into(),
            legend: true,
        };
        for name in ["figure.png", "figure.svg"] {
            write_figure(&dir.path(name), &figure).expect(name);
        }
        let svg = std::fs::read_to_string(dir.path("figure.svg")).expect("read SVG");
        assert!(svg.contains("with gaps"));
        assert!(
            !svg.contains("all NaN"),
            "curve without finite points drawn"
        );
    }

    #[test]
    fn nothing_to_draw_is_an_error_without_writing() {
        let dir = TempDir::new("empty");
        let no_curves = Figure {
            curves: Vec::new(),
            x_title: "x".into(),
            y_title: "y".into(),
            legend: false,
        };
        let no_points = one_curve("a", Vec::new());
        for figure in [&no_curves, &no_points] {
            let path = dir.path("figure.png");
            let err = write_figure(&path, figure).expect_err("must fail");
            assert!(err.starts_with(&path.display().to_string()), "{err}");
        }
        assert!(dir.entries().is_empty(), "wrote {:?}", dir.entries());
    }
}
