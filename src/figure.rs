//! Writing the displayed curves to an image file (PNG, SVG or PDF) with ruviz.
//!
//! [`write_figure`] takes plain data and a path, so it does not depend on the
//! UI and the files it writes are checked by unit tests.
//!
//! ruviz decides nothing from the file name ([`ruviz::prelude::Plot::save`]
//! always writes PNG, `export_svg` always SVG, `save_pdf` always PDF), so the
//! format is chosen here from the extension, and an unknown extension is
//! refused before anything is written.

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
pub const EXTENSIONS: &[&str] = &["png", "svg", "pdf"];

/// Line width of every curve, the same as on screen.
const LINE_WIDTH: f32 = 1.5;

// Text limits.
//
// Axis titles and legend names come from the input file (its XUNITS / YUNITS
// values and column label row) without any length or count limit, and the
// figure is written on the UI thread. ruviz's PDF export is slow in that text:
// super-linear in the length of one text (about quadratic, 0.6 s for 25 000
// characters, 33 s for 200 000) and, in the legend, about 0.9 ms per entry
// plus 0.15 ms per CJK character (ASCII is far cheaper), whatever the number
// of curves. All timings are release builds of ruviz 0.15.1 writing PDF. The
// three limits below bound the worst case and leave every real figure
// untouched: 5000 curves with CJK names of 30, 100 or 1000 characters and
// both axis titles 100 000 CJK characters long are written in 0.58-0.63 s
// (1000 curves with 200-character CJK names took 34 s without the legend
// limits, 0.54 s with them).

/// Longest axis title or legend name, in characters, that is handed to ruviz;
/// [`figure_text`] cuts longer text to this length, ending in an ellipsis.
///
/// Real units and labels are a few dozen characters, and the 640 px wide
/// figure holds only about 90 average Latin characters of ruviz's 13.9 px
/// label font (about 45 CJK ones), so nothing readable is cut. One text this
/// long takes about 20 ms.
const MAX_TEXT_CHARS: usize = 100;

/// Most legend entries handed to ruviz; [`legend_labels`] leaves the curves
/// after them without a label, so they are drawn but not listed. 100
/// one-character CJK entries take 0.1 s (1000 take 0.94 s).
const MAX_LEGEND_ENTRIES: usize = 100;

/// Most characters, summed over all legend entries, handed to ruviz;
/// [`legend_labels`] stops labelling at the first entry that would go past
/// it. 3000 CJK characters take about 0.45 s (100 entries of 30: 0.54 s; 50 of
/// 60: 0.53 s), against 34 s for 1000 entries of 200 without this budget.
const MAX_LEGEND_CHARS: usize = 3000;

/// Marks the end of a text cut by [`figure_text`].
const ELLIPSIS: char = '\u{2026}';

/// `text` as it is handed to ruviz: at most [`MAX_TEXT_CHARS`] characters,
/// cut on a `char` boundary and ending in [`ELLIPSIS`] when longer.
///
/// ruviz writes text into SVG unchanged, and the characters XML does not allow
/// (control characters other than TAB / LF / CR, and U+FFFE / U+FFFF) would
/// make the SVG invalid and the PDF export fail, so they become U+FFFD. TAB, LF
/// and CR are allowed but do not lay out as one line, so they become spaces.
fn figure_text(text: &str) -> String {
    let clean = |c: char| match c {
        '\t' | '\n' | '\r' => ' ',
        '\u{FFFE}' | '\u{FFFF}' => char::REPLACEMENT_CHARACTER,
        c if c.is_control() => char::REPLACEMENT_CHARACTER,
        c => c,
    };
    if text.chars().nth(MAX_TEXT_CHARS).is_none() {
        return text.chars().map(clean).collect();
    }
    text.chars()
        .take(MAX_TEXT_CHARS - 1)
        .map(clean)
        .chain([ELLIPSIS])
        .collect()
}

/// The legend label of each drawn curve, in order, as handed to ruviz.
///
/// Each name goes through [`figure_text`]. The labelled curves are always the
/// first ones: labelling stops for good at the first curve past
/// [`MAX_LEGEND_ENTRIES`] or whose label would take the total past
/// [`MAX_LEGEND_CHARS`], and that curve and all later ones get `None`. ruviz
/// lists a series given any label, even an empty one, so `None` means the
/// series must be given no label at all.
fn legend_labels<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Option<String>> {
    let mut total = 0;
    let mut open = true;
    names
        .into_iter()
        .enumerate()
        .map(|(k, name)| {
            let label = figure_text(name);
            total += label.chars().count();
            open = open && k < MAX_LEGEND_ENTRIES && total <= MAX_LEGEND_CHARS;
            open.then_some(label)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Svg,
    Pdf,
}

impl Format {
    /// The format named by the extension of `path`, ignoring case.
    fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }
}

/// Draws `figure` and writes it to `path`, as PNG, SVG or PDF by its extension.
///
/// Non-finite points are left out; a curve without any finite point is not
/// drawn. Fails, without writing, when the extension is not one of
/// [`EXTENSIONS`] or when no curve has a finite point. Missing parent
/// directories are created. On failure the error reads `{path}: {reason}`, like
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

    let drawn: Vec<(&FigureCurve, Vec<f64>, Vec<f64>)> = figure
        .curves
        .iter()
        .filter_map(|curve| {
            let (xs, ys): (Vec<f64>, Vec<f64>) = curve
                .points
                .iter()
                .filter(|[x, y]| x.is_finite() && y.is_finite())
                .map(|&[x, y]| (x, y))
                .unzip();
            (!xs.is_empty()).then_some((curve, xs, ys))
        })
        .collect();
    if drawn.is_empty() {
        return Err(fail("no curve with finite points to draw".into()));
    }
    let labels = legend_labels(drawn.iter().map(|(curve, _, _)| curve.name.as_str()));

    let mut plot = Plot::new()
        .xlabel(figure_text(&figure.x_title))
        .ylabel(figure_text(&figure.y_title));
    for ((curve, xs, ys), label) in drawn.iter().zip(labels) {
        let [r, g, b, a] = curve.color.to_srgba_unmultiplied();
        let series = plot.line(xs, ys);
        let series = match label {
            Some(label) => series.label(label),
            None => series,
        };
        plot = series
            .color(Color::from_rgba(r, g, b, a))
            .line_width(LINE_WIDTH)
            .into();
    }
    if figure.legend {
        plot = plot.legend_best();
    }

    match format {
        Format::Png => plot.save(path),
        Format::Svg => plot.export_svg(path),
        // ruviz's default page, 160 x 120 mm.
        Format::Pdf => plot.save_pdf(path),
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
    const PDF_SIGNATURE: &[u8] = b"%PDF-";

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
    fn writes_fixture_as_pdf() {
        let dir = TempDir::new("pdf");
        let path = dir.path("figure.pdf");
        write_figure(&path, &fixture_figure()).expect("write PDF");
        let bytes = std::fs::read(&path).expect("read PDF");
        assert!(bytes.starts_with(PDF_SIGNATURE), "not a PDF file");
        // ruviz writes through a temporary sibling file; none may be left.
        assert_eq!(dir.entries(), ["figure.pdf"]);
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
        let pdf = dir.path("figure.PDF");
        write_figure(&png, &figure).expect("write .PNG");
        write_figure(&svg, &figure).expect("write .Svg");
        write_figure(&pdf, &figure).expect("write .PDF");
        assert!(std::fs::read(&png).unwrap().starts_with(PNG_SIGNATURE));
        assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
        assert!(std::fs::read(&pdf).unwrap().starts_with(PDF_SIGNATURE));
        assert_eq!(dir.entries(), ["figure.PDF", "figure.PNG", "figure.Svg"]);
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
            assert!(err.contains("(supported: .png / .svg / .pdf)"), "{err}");
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
    fn very_long_legend_name_and_axis_title_are_shortened() {
        let dir = TempDir::new("long-text");
        // Far beyond the cap, multi-byte, as from a crafted file's header.
        let long_name = "合成".repeat(100_000);
        let long_x = "nm".repeat(100_000);
        let mut figure = one_curve(&long_name, vec![[1.0, 1.0], [2.0, 3.0]]);
        figure.x_title = long_x.clone();

        let svg_path = dir.path("figure.svg");
        write_figure(&svg_path, &figure).expect("write SVG");
        let svg = std::fs::read_to_string(&svg_path).expect("read SVG");
        let shortened =
            |text: &str| text.chars().take(MAX_TEXT_CHARS - 1).collect::<String>() + "…";
        let short_name = shortened(&long_name);
        let short_x = shortened(&long_x);
        assert!(
            svg.contains(&short_name),
            "SVG lacks the shortened legend name"
        );
        assert!(svg.contains(&short_x), "SVG lacks the shortened x title");
        assert!(
            !svg.contains(&long_x[..2 * MAX_TEXT_CHARS]),
            "x title not shortened"
        );

        let pdf_path = dir.path("figure.pdf");
        write_figure(&pdf_path, &figure).expect("write PDF");
        let bytes = std::fs::read(&pdf_path).expect("read PDF");
        assert!(bytes.starts_with(PDF_SIGNATURE), "not a PDF file");

        let png_path = dir.path("figure.png");
        write_figure(&png_path, &figure).expect("write PNG");
        let bytes = std::fs::read(&png_path).expect("read PNG");
        assert!(bytes.starts_with(PNG_SIGNATURE), "not a PNG file");
    }

    #[test]
    fn figure_text_keeps_short_text_and_cuts_long_text_on_char_boundaries() {
        for text in ["", "Wavelength [nm]", "合成試料その一 25℃"] {
            assert_eq!(figure_text(text), text);
        }
        let at_cap = "あ".repeat(MAX_TEXT_CHARS);
        assert_eq!(figure_text(&at_cap), at_cap);

        let over_cap = "あ".repeat(MAX_TEXT_CHARS + 1);
        let cut = figure_text(&over_cap);
        assert_eq!(cut.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(cut, "あ".repeat(MAX_TEXT_CHARS - 1) + "…");

        // A base letter and its combining mark are two chars; cutting between
        // them must not panic, and the result stays at the cap.
        let combining = "e\u{301}".repeat(MAX_TEXT_CHARS);
        let cut = figure_text(&combining);
        assert_eq!(cut.chars().count(), MAX_TEXT_CHARS);
        assert!(cut.ends_with('…'));
        assert!(combining.starts_with(cut.trim_end_matches('…')));
    }

    #[test]
    fn control_characters_are_replaced() {
        assert_eq!(
            figure_text("a\0b\x07c\u{FFFE}d\u{FFFF}e\x7f"),
            "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}"
        );
        assert_eq!(figure_text("a\tb\nc\r\nd"), "a b c  d");

        let dir = TempDir::new("control");
        let mut figure = one_curve("a\0b\u{FFFE}", vec![[1.0, 1.0], [2.0, 3.0]]);
        figure.x_title = "nm\u{FFFF}".into();
        figure.y_title = "Abs\x07".into();
        let svg_path = dir.path("figure.svg");
        write_figure(&svg_path, &figure).expect("write SVG");
        let svg = std::fs::read_to_string(&svg_path).expect("read SVG");
        assert!(
            !svg.contains(['\0', '\x07', '\u{FFFE}', '\u{FFFF}']),
            "character not allowed in XML written to SVG"
        );
        assert!(
            svg.contains("a\u{FFFD}b\u{FFFD}"),
            "SVG lacks the legend name"
        );
        assert!(svg.contains("nm\u{FFFD}"), "SVG lacks the x title");
        assert!(svg.contains("Abs\u{FFFD}"), "SVG lacks the y title");

        let pdf_path = dir.path("figure.pdf");
        write_figure(&pdf_path, &figure).expect("write PDF");
        assert!(std::fs::read(&pdf_path).unwrap().starts_with(PDF_SIGNATURE));
    }

    #[test]
    fn legend_labels_keep_a_normal_figure_unchanged() {
        let names = ["sample A", "合成試料その一 25℃", "Blank"];
        let labels = legend_labels(names);
        let expected: Vec<Option<String>> = names.iter().map(|n| Some(n.to_string())).collect();
        assert_eq!(labels, expected);
    }

    /// Checks the legend limits on `labels`: the labelled entries come first
    /// and stay within the entry count and the total character budget.
    fn assert_legend_bounded(labels: &[Option<String>]) -> usize {
        let labelled = labels.iter().take_while(|l| l.is_some()).count();
        assert!(
            labels[labelled..].iter().all(Option::is_none),
            "a label after the first unlabelled curve"
        );
        assert!(labelled <= MAX_LEGEND_ENTRIES, "{labelled} entries");
        let total: usize = labels.iter().flatten().map(|l| l.chars().count()).sum();
        assert!(total <= MAX_LEGEND_CHARS, "{total} legend characters");
        labelled
    }

    #[test]
    fn legend_labels_bound_the_total_legend_text() {
        // Many long CJK names: the character budget stops the labels.
        let long = "合".repeat(10_000);
        let labels = legend_labels(std::iter::repeat_n(long.as_str(), 1000));
        assert_eq!(labels.len(), 1000);
        let labelled = assert_legend_bounded(&labels);
        assert_eq!(labelled, MAX_LEGEND_CHARS / MAX_TEXT_CHARS);
        assert_eq!(labels[0].as_deref(), Some(figure_text(&long).as_str()));

        // Many one-character names: the entry count stops the labels.
        let labels = legend_labels(std::iter::repeat_n("合", 5000));
        assert_eq!(assert_legend_bounded(&labels), MAX_LEGEND_ENTRIES);

        // A name that does not fit ends the labels, even when a later, shorter
        // one would still fit: 30 x 99 = 2970, then 100 does not fit, 1 would.
        let mut names = vec!["合".repeat(99); 30];
        names.push("合".repeat(MAX_TEXT_CHARS));
        names.push("合".into());
        let labels = legend_labels(names.iter().map(String::as_str));
        assert_eq!(assert_legend_bounded(&labels), 30);
        assert_eq!(labels.len(), 32);
    }

    #[test]
    fn many_long_cjk_names_give_a_bounded_svg_legend() {
        let dir = TempDir::new("many-names");
        let figure = Figure {
            curves: (0..1000)
                .map(|k| FigureCurve {
                    name: "合".repeat(1000),
                    points: vec![[1.0, k as f64], [2.0, k as f64 + 1.0]],
                    color: Color32::RED,
                })
                .collect(),
            x_title: "Wavelength [nm]".into(),
            y_title: "Abs".into(),
            legend: true,
        };
        let path = dir.path("figure.svg");
        write_figure(&path, &figure).expect("write SVG");
        let svg = std::fs::read_to_string(&path).expect("read SVG");
        let cjk = svg.chars().filter(|&c| c == '合').count();
        assert!(cjk > 0, "no legend text in SVG");
        assert!(cjk <= MAX_LEGEND_CHARS, "{cjk} legend characters in SVG");
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
