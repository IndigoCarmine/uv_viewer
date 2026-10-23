//! Writing the displayed curves to an image file (PNG, SVG or PDF) with ruviz.
//!
//! [`write_figure`] takes plain data and a path, so it does not depend on the
//! UI and the files it writes are checked by unit tests.
//!
//! ruviz decides nothing from the file name ([`ruviz::prelude::Plot::save`]
//! always writes PNG, `export_svg` always SVG, `save_pdf` always PDF), so the
//! format is chosen here from the extension, and an unknown extension is
//! refused before anything is written.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use egui::Color32;
use ruviz::core::{Legend, LegendItem, LegendSpacing, measure_legend_size};
use ruviz::export::page_sizes;
use ruviz::prelude::{Color, FontConfig, FontFamily, LegendPosition, LineStyle, Plot};
use ruviz::render::TextRenderer;

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
// figure is written on the UI thread. ruviz's export is slow in that text:
// super-linear in the length of one text (about quadratic, 0.6 s for 25 000
// characters, 33 s for 200 000) and, in the legend, linear in the number of
// entries and of characters at a cost that depends on the characters. The
// most expensive are characters the fonts have no glyph for, such as
// unassigned code points (U+3FF00..) and, close behind, emoji: about 3 ms per
// legend entry and 1 ms per character. CJK Extension B and varied
// (non-repeating) common CJK cost less, ASCII almost nothing; the numbers for
// each are below. All timings are PDF, release build, ruviz 0.15.1.
//
// The three limits below were chosen by timing [`write_figure`] at them, with
// both axis titles 100 000 characters long (so cut to [`MAX_TEXT_CHARS`]),
// curves of 300 points, and every curve named with 1, 12, 13, 30, 100 or 1000
// characters of one class: so the legend is either [`MAX_LEGEND_ENTRIES`]
// short entries, or as many entries as fit in [`MAX_LEGEND_CHARS`]. Slowest
// of those per class, with 100 curves: unassigned 0.82 s, emoji 0.69 s, CJK
// Extension B 0.37 s, varied CJK 0.15 s, ASCII 0.06 s. With 1000 curves:
// unassigned 1.25 s, emoji 1.03 s, CJK Extension B 0.71 s, varied CJK 0.50 s,
// ASCII 0.47 s; of that, about 0.38 s is drawing the 300 000 points, whatever
// the text.
//
// The price is that a figure with more than 40 curves, or with long names
// (for example more than 20 of 25 characters), lists only its first curves in
// the legend; the others are drawn without a legend entry, with no warning.

/// Longest axis title or legend name, in characters, that is handed to ruviz;
/// [`figure_text`] cuts longer text to this length, ending in an ellipsis.
///
/// Real units and labels are a few dozen characters, and the 640 px wide
/// figure holds only about 90 average Latin characters of ruviz's 13.9 px
/// label font (about 45 CJK ones), so nothing readable is cut.
const MAX_TEXT_CHARS: usize = 100;

/// Most legend entries handed to ruviz; [`legend_labels`] leaves the curves
/// after them without a label, so they are drawn but not listed. Inside the
/// axes, ruviz's legend runs off the figure well before this many entries, so
/// [`fit_legend`] moves a legend this long next to the axes.
const MAX_LEGEND_ENTRIES: usize = 40;

/// Most characters, summed over all legend entries, handed to ruviz;
/// [`legend_labels`] stops labelling at the first entry that would go past
/// it. Without this budget, 1000 curves with 100 entries of 10 unassigned code
/// points took 2.0 s; with it, the slowest case at the limits is the 1.25 s
/// above, 0.38 s of it the curves.
const MAX_LEGEND_CHARS: usize = 500;

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

// Legend layout.
//
// ruviz lays the legend out in the columns it is told (one by default) and
// never wraps, shrinks or drops entries of a legend it places itself:
// `legend_best` puts it at one of nine places inside the axes, and a legend
// taller or wider than the axes runs over the axis labels and off the canvas
// (24 entries already lose their last two in a 640 x 480 PNG). The canvas does
// not grow for it either. So [`fit_legend`] decides, from the size the legend
// will have, between ruviz's default and a legend next to the axes on a
// canvas widened for it.
//
// All sizes are in pixels at ruviz's default 100 dpi, which [`write_figure`]
// keeps; text sizes are in points, so they do not change with the canvas.

/// Canvas of a PNG or SVG figure: ruviz's default figure, 6.4 x 4.8 in.
const IMAGE_CANVAS: (u32, u32) = (640, 480);

/// Legend font size: ruviz's default legend font, 9 pt.
const LEGEND_FONT_PX: f32 = 9.0 * 100.0 / 72.0;

/// Height of the canvas that is not axes, measured in ruviz's SVG (6.94 px
/// above the axes, 49.03 px below for the x tick labels and title, the same
/// on every canvas as it depends only on the font sizes), rounded up.
const NON_AXES_HEIGHT_PX: f32 = 56.0;

/// Where the legend goes, decided by [`fit_legend`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegendFit {
    /// ruviz's default: one column at the best place inside the axes, on the
    /// default canvas.
    Inside,
    /// To the right of the axes, in `columns` columns of entries, on a canvas
    /// widened to `width` pixels; the canvas height is the default one.
    Outside { columns: usize, width: u32 },
}

/// Where a legend listing `labels` goes on a figure whose default canvas is
/// `canvas` (width, height) pixels; `measure` gives the width of a label as
/// ruviz draws it in the legend.
///
/// The legend stays where ruviz puts it by default ([`LegendFit::Inside`])
/// when, in one column, it is no wider than half the canvas and no taller than
/// the axes less ruviz's gap to them. The height test is what the axes hold:
/// 19 entries in a PNG or SVG (424 px of axes), 17 in a PDF (397 px). The width
/// test is tighter than the axes, whose width ruviz decides only while
/// drawing, from the y tick labels: half the canvas also leaves the legend
/// inside the narrowest axes measured, 391 px for 13-digit tick labels. So a
/// figure with up to that many entries keeps its canvas and legend unless a
/// name is wider than about 270 px in a PNG or SVG, 250 px in a PDF (roughly
/// 45 or 40 Latin characters, 21 or 19 CJK ones).
///
/// Any other legend goes outside the axes, on their right
/// ([`LegendFit::Outside`]), in the fewest columns whose height fits on the
/// default canvas height with ruviz's gap above and below: one column holds 21
/// entries in a PNG or SVG and 19 in a PDF, so [`MAX_LEGEND_ENTRIES`] entries
/// take 2 and 3 columns. The canvas is widened by the legend and its gaps, so
/// the axes keep their default width. ruviz measures the legend itself when it
/// reserves that room and gives the axes whatever is left, so the legend stays
/// on the canvas even if `measure` is a little off. The widest legend, one
/// name of [`MAX_TEXT_CHARS`] CJK characters among [`MAX_LEGEND_ENTRIES`]
/// entries, widens a PNG to about 3300 px (two columns) and a PDF to about
/// 4600 px (three).
fn fit_legend(
    canvas: (u32, u32),
    labels: &[&str],
    mut measure: impl FnMut(&str) -> ruviz::core::Result<f32>,
) -> ruviz::core::Result<LegendFit> {
    let (width, height) = (canvas.0 as f32, canvas.1 as f32);
    let items: Vec<LegendItem> = labels
        .iter()
        .map(|label| LegendItem::line(*label, Color::BLACK, LineStyle::Solid, LINE_WIDTH))
        .collect();
    // ruviz's own formula, with its default spacing, as it sizes the legend.
    let mut size = |columns: usize| {
        let legend = Legend {
            font_size: LEGEND_FONT_PX,
            columns,
            ..Legend::default()
        };
        measure_legend_size(&items, &legend, &mut measure)
    };
    // Gap between the legend and the axes or the canvas edge.
    let pad = LegendSpacing::default()
        .to_pixels(LEGEND_FONT_PX)
        .border_axes_pad;

    let (one_column_width, one_column_height) = size(1)?;
    if one_column_width <= width / 2.0 && one_column_height + pad <= height - NON_AXES_HEIGHT_PX {
        return Ok(LegendFit::Inside);
    }
    let mut columns = 1;
    let legend_width = loop {
        let (legend_width, legend_height) = size(columns)?;
        // One row always fits; the bound only ends the loop.
        if legend_height + 2.0 * pad <= height || columns >= labels.len() {
            break legend_width;
        }
        columns += 1;
    };
    Ok(LegendFit::Outside {
        columns,
        width: canvas.0 + (legend_width + 2.0 * pad).ceil() as u32,
    })
}

/// [`fit_legend`] for a `format` figure whose curves have `labels`, as from
/// [`legend_labels`], with the labels measured as ruviz measures them: by its
/// text renderer, in the default sans-serif family and text options of its
/// typography.
fn legend_fit(format: Format, labels: &[Option<String>]) -> ruviz::core::Result<LegendFit> {
    let listed: Vec<&str> = labels.iter().flatten().map(String::as_str).collect();
    let text = TextRenderer::new();
    let font = FontConfig::new(FontFamily::SansSerif, LEGEND_FONT_PX);
    fit_legend(format.canvas(), &listed, |label| {
        Ok(text.measure_text(label, &font)?.0)
    })
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

    /// The default canvas, (width, height) in pixels. A PDF is drawn on its
    /// page, ruviz's default `PLOT_DEFAULT` of 160 x 120 mm turned into pixels
    /// at 96 per inch (604 x 453 px).
    fn canvas(self) -> (u32, u32) {
        match self {
            Self::Png | Self::Svg => IMAGE_CANVAS,
            Self::Pdf => {
                let (width, height) = page_sizes::PLOT_DEFAULT;
                (
                    page_sizes::mm_to_px(width) as u32,
                    page_sizes::mm_to_px(height) as u32,
                )
            }
        }
    }
}

/// Page width, in mm, that ruviz turns into a canvas `width` pixels wide: it
/// truncates `mm_to_px`, so half a pixel more keeps rounding error from
/// taking a pixel off.
fn pdf_page_width_mm(width: u32) -> f64 {
    (f64::from(width) + 0.5) * 25.4 / 96.0
}

/// Draws `figure` and writes it to `path`, as PNG, SVG or PDF by its extension.
///
/// Non-finite points are left out; a curve without any finite point is not
/// drawn. Fails, without writing, when the extension is not one of
/// [`EXTENSIONS`] or when no curve has a finite point. Missing parent
/// directories are created. An existing file at `path` is replaced; when it
/// cannot be (it is read-only, or held open on Windows), it is left as it was
/// and no other file is left behind. A symbolic link at `path` is itself
/// replaced, not the file it points to. On failure the error reads
/// `{path}: {reason}`, like [`crate::jasco::load`].
///
/// A PNG or SVG is 640 x 480 px and a PDF page ruviz's default (604 x 453
/// px, written as a 604 x 453 pt page), unless the legend is too large for the
/// axes (see [`fit_legend`]): then the legend is drawn to the right of the
/// axes and the figure is that many pixels wider, at the same height. A PDF
/// page is then wider by as many points, with the plot and text at their usual
/// size, so a document that places the figure at a fixed width shows them
/// smaller.
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
    let fit = if figure.legend {
        Some(legend_fit(format, &labels).map_err(|e| fail(e.to_string()))?)
    } else {
        None
    };

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
    // Width of a canvas widened for the legend; `None` keeps the default.
    let mut widened = None;
    match fit {
        None => {}
        Some(LegendFit::Inside) => plot = plot.legend_best(),
        Some(LegendFit::Outside { columns, width }) => {
            plot = plot
                .legend(LegendPosition::OutsideRight)
                .legend_columns(columns);
            widened = Some(width);
        }
    }
    if let (Some(width), Format::Png | Format::Svg) = (widened, format) {
        plot = plot.size_px(width, IMAGE_CANVAS.1);
    }

    // ruviz writes through a temporary sibling of the path it is given and
    // keeps that file when the final replace fails (on Windows, for a
    // read-only destination or one another program holds open), so it would
    // pile up next to the user's figure on every retry. ruviz is therefore
    // given a new file of ours, which its replace never fails on, and that
    // file is moved over `path` here and removed if anything fails.
    let temp = create_temp(path).map_err(|e| fail(e.to_string()))?;
    let result = for_ruviz(&temp)
        .map_err(|e| e.to_string())
        .and_then(|target| {
            match format {
                Format::Png => plot.save(&target),
                Format::Svg => plot.export_svg(&target),
                // ruviz's default page, or one as much wider as the canvas.
                Format::Pdf => match widened {
                    None => plot.save_pdf(&target),
                    Some(width) => plot.save_pdf_with_size(
                        &target,
                        Some((pdf_page_width_mm(width), page_sizes::PLOT_DEFAULT.1)),
                    ),
                },
            }
            .map_err(|e| e.to_string())
        })
        .and_then(|()| std::fs::rename(&temp, path).map_err(|e| e.to_string()));
    if result.is_err() {
        // The write error is what is reported.
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(fail)
}

/// Most names [`create_temp`] tries before giving up.
const TEMP_ATTEMPTS: u32 = 100;

/// Creates a new, empty file in the directory of `path`, creating missing
/// directories, for the figure to be written to before it is moved over
/// `path`, and returns its path. Being in the same directory, the move is a
/// rename on one volume.
///
/// The name is `.uvv-{pid}-{n}.tmp`, `n` counting up in this process, and the
/// file is created only if no file has that name yet, so nothing that exists
/// is overwritten or followed. The name is short and does not grow with the
/// file name of `path`, as ruviz names its own temporary file after it (see
/// [`for_ruviz`]).
fn create_temp(path: &Path) -> std::io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_file_name(format!(".uvv-{}-{n}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(_) => return Ok(temp),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("TEMP_ATTEMPTS is not 0"))
}

/// `temp` as it is handed to ruviz.
///
/// On Windows, ruviz writes `.{file name}.{pid}.{n}.tmp` next to the path it
/// is given and moves it into place with MoveFileExW, which fails once either
/// path reaches MAX_PATH (260 UTF-16 units) unless it is a `\\?\` path; the
/// standard library makes long paths `\\?\` ones itself, ruviz does not. A
/// temporary name of ours that is longer than the destination's name would
/// make that happen for shorter destinations than before, so ruviz is given
/// the absolute `\\?\` form of `temp`, for which there is no such limit.
#[cfg(windows)]
fn for_ruviz(temp: &Path) -> std::io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Prefix};

    // Resolves `.`, `..` and `/`, which a `\\?\` path must not contain.
    let absolute = std::path::absolute(temp)?;
    let wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let (prefix, rest) = match absolute.components().next() {
        // `C:\dir` becomes `\\?\C:\dir`.
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)) => (r"\\?\", &wide[..]),
        // `\\server\share\dir` becomes `\\?\UNC\server\share\dir`.
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::UNC(..)) => {
            (r"\\?\UNC", &wide[1..])
        }
        // Already a `\\?\` or device path.
        _ => return Ok(absolute),
    };
    let mut verbatim: Vec<u16> = prefix.encode_utf16().collect();
    verbatim.extend_from_slice(rest);
    Ok(OsString::from_wide(&verbatim).into())
}

/// `temp` as it is handed to ruviz: unchanged, as only Windows limits ruviz's
/// path length.
#[cfg(not(windows))]
fn for_ruviz(temp: &Path) -> std::io::Result<PathBuf> {
    Ok(temp.to_path_buf())
}

#[cfg(test)]
mod tests {
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
        // one would still fit: n names of `len`, then one of MAX_TEXT_CHARS
        // that goes past the budget, then one character that would fit.
        let len = MAX_TEXT_CHARS - 1;
        let n = MAX_LEGEND_CHARS / len;
        assert!(n + 2 <= MAX_LEGEND_ENTRIES);
        assert!(n * len + MAX_TEXT_CHARS > MAX_LEGEND_CHARS);
        assert!(n * len < MAX_LEGEND_CHARS);
        let mut names = vec!["合".repeat(len); n];
        names.push("合".repeat(MAX_TEXT_CHARS));
        names.push("合".into());
        let labels = legend_labels(names.iter().map(String::as_str));
        assert_eq!(assert_legend_bounded(&labels), n);
        assert_eq!(labels.len(), n + 2);
    }

    #[test]
    fn curves_past_the_legend_limit_add_no_legend_entry() {
        let figure = |curves: usize| Figure {
            curves: (0..curves)
                .map(|k| FigureCurve {
                    name: format!("c{k}"),
                    // The same points, so the axes and ticks do not change.
                    points: vec![[1.0, 1.0], [2.0, 3.0]],
                    color: Color32::RED,
                })
                .collect(),
            x_title: "x".into(),
            y_title: "y".into(),
            legend: true,
        };
        let dir = TempDir::new("legend-entries");
        let svg_of = |figure: &Figure, name: &str| {
            let path = dir.path(name);
            write_figure(&path, figure).expect("write SVG");
            std::fs::read_to_string(&path).expect("read SVG")
        };
        let listed = svg_of(&figure(MAX_LEGEND_ENTRIES), "listed.svg");
        let more = svg_of(&figure(MAX_LEGEND_ENTRIES + 5), "more.svg");
        let last = format!(">c{}<", MAX_LEGEND_ENTRIES - 1);
        assert!(more.contains(&last), "SVG lacks the last listed entry");
        assert!(!more.contains(&format!(">c{MAX_LEGEND_ENTRIES}<")));
        // A curve given an empty label would still add an (empty) text.
        assert_eq!(
            more.matches("<text").count(),
            listed.matches("<text").count(),
            "curves past the limit changed the legend"
        );
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

    /// A number attribute `name="…"` of the SVG element that starts at `tag`.
    fn svg_attr(tag: &str, name: &str) -> f32 {
        let key = format!(" {name}=\"");
        let start = tag
            .find(&key)
            .unwrap_or_else(|| panic!("no {name} in {tag}"))
            + key.len();
        let len = tag[start..].find('"').expect("closing quote");
        tag[start..start + len].parse().expect("number")
    }

    /// Width and height of the SVG canvas.
    fn svg_canvas(svg: &str) -> (f32, f32) {
        let tag = &svg[svg.find("<svg").expect("<svg")..];
        (svg_attr(tag, "width"), svg_attr(tag, "height"))
    }

    /// `(x, y)` of the `<text>` element whose content is exactly `content`.
    fn svg_text_position(svg: &str, content: &str) -> (f32, f32) {
        let end = svg
            .find(&format!(">{content}</text>"))
            .unwrap_or_else(|| panic!("SVG lacks the text {content:?}"));
        let tag = &svg[svg[..end].rfind("<text ").expect("<text")..end];
        (svg_attr(tag, "x"), svg_attr(tag, "y"))
    }

    /// `(left, top, right, bottom)` of the legend frame, the only rounded,
    /// translucent rectangle ruviz draws.
    fn svg_legend_frame(svg: &str) -> (f32, f32, f32, f32) {
        let at = svg
            .find("fill=\"rgba(255,255,255,0.800)\"")
            .expect("SVG has no legend frame");
        let tag = &svg[svg[..at].rfind("<rect ").expect("<rect")..at];
        let (x, y) = (svg_attr(tag, "x"), svg_attr(tag, "y"));
        (
            x,
            y,
            x + svg_attr(tag, "width"),
            y + svg_attr(tag, "height"),
        )
    }

    /// Width and height of a PNG file, from its IHDR chunk.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(PNG_SIGNATURE), "not a PNG file");
        let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        (be(16), be(20))
    }

    /// One curve for each of `names`, all with the same points, so the axes
    /// do not change with the names.
    fn named_curves(names: impl IntoIterator<Item = String>) -> Figure {
        Figure {
            curves: names
                .into_iter()
                .map(|name| FigureCurve {
                    name,
                    points: vec![[200.0, 0.1], [800.0, 1.2]],
                    color: Color32::RED,
                })
                .collect(),
            x_title: "Wavelength [nm]".into(),
            y_title: "Abs".into(),
            legend: true,
        }
    }

    /// Checks that every legend entry of `figure` is in the SVG and on its
    /// canvas, and that the legend frame is too.
    fn assert_svg_legend_on_canvas(figure: &Figure, svg: &str) {
        let (width, height) = svg_canvas(svg);
        let (left, top, right, bottom) = svg_legend_frame(svg);
        assert!(
            left >= 0.0 && top >= 0.0 && right <= width && bottom <= height,
            "legend frame {left}..{right} x {top}..{bottom} off the {width} x {height} canvas"
        );
        let labels = legend_labels(figure.curves.iter().map(|c| c.name.as_str()));
        for label in labels.iter().flatten() {
            let (x, y) = svg_text_position(svg, label);
            assert!(
                (left..=right).contains(&x) && (top..=bottom).contains(&y),
                "legend entry {label:?} at ({x}, {y}) outside the frame"
            );
        }
    }

    #[test]
    fn every_legend_entry_of_many_curves_is_on_the_canvas() {
        let dir = TempDir::new("legend-fit");
        let cases = [
            // The most entries, with short names.
            named_curves((0..MAX_LEGEND_ENTRIES + 5).map(|k| format!("sample {k}"))),
            // The longest names, as many as the character budget allows.
            named_curves(
                (0..MAX_LEGEND_CHARS / MAX_TEXT_CHARS)
                    .map(|k| format!("{k}").repeat(MAX_TEXT_CHARS / 2) + &"合".repeat(50)),
            ),
            // Many entries, one of them as long as a name can be.
            named_curves(
                std::iter::once("合".repeat(MAX_TEXT_CHARS))
                    .chain((1..MAX_LEGEND_ENTRIES).map(|k| format!("s{k}"))),
            ),
        ];
        for (k, figure) in cases.iter().enumerate() {
            let svg_path = dir.path(&format!("figure{k}.svg"));
            write_figure(&svg_path, figure).expect("write SVG");
            let svg = std::fs::read_to_string(&svg_path).expect("read SVG");
            assert_svg_legend_on_canvas(figure, &svg);
            // Beside the axes, at the default height, the canvas widened by
            // the legend as ruviz measured it and its two 12.5 px gaps: so
            // the labels are measured here as ruviz measures them.
            let (width, height) = svg_canvas(&svg);
            let (left, _, right, _) = svg_legend_frame(&svg);
            assert_eq!(height, IMAGE_CANVAS.1 as f32);
            assert!(left > IMAGE_CANVAS.0 as f32 / 2.0, "legend left at {left}");
            let expected = IMAGE_CANVAS.0 as f32 + (right - left + 25.0).ceil();
            assert!(
                (width - expected).abs() <= 1.0,
                "{width} px, not {expected}"
            );

            // PNG is laid out by the same code at the same size.
            let png_path = dir.path(&format!("figure{k}.png"));
            write_figure(&png_path, figure).expect("write PNG");
            assert_eq!(
                png_size(&std::fs::read(&png_path).unwrap()),
                (width as u32, height as u32)
            );

            // PDF: ruviz draws the same SVG on a canvas the page size sets.
            let pdf_path = dir.path(&format!("figure{k}.pdf"));
            write_figure(&pdf_path, figure).expect("write PDF");
            let labels = legend_labels(figure.curves.iter().map(|c| c.name.as_str()));
            let LegendFit::Outside { width, .. } = legend_fit(Format::Pdf, &labels).unwrap() else {
                panic!("PDF legend left inside the axes");
            };
            let page = pdf_media_box(&std::fs::read(&pdf_path).unwrap());
            assert_eq!(page, (width, Format::Pdf.canvas().1));
        }
    }

    /// Width and height of the page of a one-page PDF written by ruviz, which
    /// gives its MediaBox in whole units of its canvas pixels.
    fn pdf_media_box(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(PDF_SIGNATURE), "not a PDF file");
        let text = String::from_utf8_lossy(bytes);
        let start = text.find("/MediaBox [").expect("no MediaBox") + "/MediaBox [".len();
        let numbers: Vec<u32> = text[start..start + text[start..].find(']').unwrap()]
            .split_whitespace()
            .map(|n| n.parse().expect("whole number"))
            .collect();
        assert_eq!(numbers[..2], [0, 0]);
        (numbers[2], numbers[3])
    }

    /// `figure`, all of whose points are finite and whose names fit the
    /// legend limits, as [`write_figure`] drew it before it fitted the
    /// legend: ruviz's default canvas and its best place inside the axes.
    fn default_layout_plot(figure: &Figure) -> Plot {
        let mut plot = Plot::new()
            .xlabel(figure_text(&figure.x_title))
            .ylabel(figure_text(&figure.y_title));
        for curve in &figure.curves {
            let (xs, ys): (Vec<f64>, Vec<f64>) = curve.points.iter().map(|&[x, y]| (x, y)).unzip();
            let [r, g, b, a] = curve.color.to_srgba_unmultiplied();
            plot = plot
                .line(&xs, &ys)
                .label(figure_text(&curve.name))
                .color(Color::from_rgba(r, g, b, a))
                .line_width(LINE_WIDTH)
                .into();
        }
        plot.legend_best()
    }

    #[test]
    fn a_legend_the_axes_hold_keeps_the_default_figure() {
        let dir = TempDir::new("legend-default");
        let names = |n: usize| (0..n).map(|k| format!("sample-{k}.txt Abs"));
        let read = |name: &str| std::fs::read(dir.path(name)).expect("read figure");

        // The most entries the axes hold: 19 in a PNG or SVG, 17 in a PDF.
        for figure in [fixture_figure(), named_curves(names(19))] {
            for name in ["figure.svg", "figure.png"] {
                write_figure(&dir.path(name), &figure).expect(name);
            }
            default_layout_plot(&figure)
                .export_svg(dir.path("default.svg"))
                .expect("write default SVG");
            default_layout_plot(&figure)
                .save(dir.path("default.png"))
                .expect("write default PNG");
            assert!(read("figure.svg") == read("default.svg"), "SVG changed");
            assert!(read("figure.png") == read("default.png"), "PNG changed");
            assert_eq!(png_size(&read("figure.png")), IMAGE_CANVAS);
        }
        let figure = named_curves(names(17));
        write_figure(&dir.path("figure.pdf"), &figure).expect("write PDF");
        default_layout_plot(&figure)
            .save_pdf(dir.path("default.pdf"))
            .expect("write default PDF");
        assert!(read("figure.pdf") == read("default.pdf"), "PDF changed");

        // The legend font is the one the labels are measured in.
        let svg = std::fs::read_to_string(dir.path("figure.svg")).unwrap();
        let end = svg.find(">sample-0.txt Abs<").expect("legend entry");
        let tag = &svg[svg[..end].rfind("<text ").unwrap()..end];
        assert_eq!(svg_attr(tag, "font-size"), LEGEND_FONT_PX);

        // One entry more than the axes hold moves the legend out.
        write_figure(&dir.path("more.svg"), &named_curves(names(20))).expect("write SVG");
        let more = std::fs::read_to_string(dir.path("more.svg")).unwrap();
        assert!(svg_canvas(&more).0 > IMAGE_CANVAS.0 as f32);
        write_figure(&dir.path("more.pdf"), &named_curves(names(18))).expect("write PDF");
        assert!(pdf_media_box(&read("more.pdf")).0 > Format::Pdf.canvas().0);
    }

    #[test]
    fn fit_legend_moves_a_tall_or_wide_legend_beside_the_axes() {
        let fit = |canvas, n: usize, label_width: f32| {
            let labels = vec!["x"; n];
            fit_legend(canvas, &labels, |_| Ok(label_width)).unwrap()
        };
        let image = IMAGE_CANVAS;
        let pdf = Format::Pdf.canvas();
        assert_eq!(pdf, (604, 453));
        // With 20 px labels, a legend is 7.5 px of padding on either side and
        // columns of 57.5 px (line, gap and label) 25 px apart. Beside the
        // axes it takes 12.5 px of gap on either side.
        let outside = |columns: usize, canvas: (u32, u32)| {
            let legend = 15.0 + 57.5 * columns as f32 + 25.0 * (columns - 1) as f32;
            LegendFit::Outside {
                columns,
                width: canvas.0 + (legend + 25.0).ceil() as u32,
            }
        };
        for n in [0, 1, 19] {
            assert_eq!(fit(image, n, 20.0), LegendFit::Inside, "{n} entries");
        }
        assert_eq!(fit(image, 20, 20.0), outside(1, image));
        assert_eq!(fit(image, 21, 20.0), outside(1, image));
        assert_eq!(fit(image, 22, 20.0), outside(2, image));
        assert_eq!(fit(image, MAX_LEGEND_ENTRIES, 20.0), outside(2, image));
        assert_eq!(fit(pdf, 17, 20.0), LegendFit::Inside);
        assert_eq!(fit(pdf, 18, 20.0), outside(1, pdf));
        assert_eq!(fit(pdf, 19, 20.0), outside(1, pdf));
        assert_eq!(fit(pdf, 20, 20.0), outside(2, pdf));
        assert_eq!(fit(pdf, MAX_LEGEND_ENTRIES, 20.0), outside(3, pdf));

        // A legend wider than half the canvas goes out however short it is:
        // 52.5 + 267.5 = 320 px still fits a 640 px canvas.
        assert_eq!(fit(image, 1, 267.5), LegendFit::Inside);
        assert_eq!(
            fit(image, 1, 268.0),
            LegendFit::Outside {
                columns: 1,
                width: 640 + 346
            }
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

    #[test]
    fn existing_figure_is_replaced() {
        let dir = TempDir::new("replace");
        let figure = one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]);
        for name in ["figure.png", "figure.svg", "figure.pdf"] {
            let path = dir.path(name);
            std::fs::write(&path, b"old").expect("write old file");
            write_figure(&path, &figure).expect("replace figure");
            assert_ne!(std::fs::read(&path).unwrap(), b"old", "{name} not replaced");
        }
        assert!(
            std::fs::read(dir.path("figure.png"))
                .unwrap()
                .starts_with(PNG_SIGNATURE)
        );
        assert!(
            std::fs::read_to_string(dir.path("figure.svg"))
                .unwrap()
                .contains("<svg")
        );
        assert!(
            std::fs::read(dir.path("figure.pdf"))
                .unwrap()
                .starts_with(PDF_SIGNATURE)
        );
        assert_eq!(dir.entries(), ["figure.pdf", "figure.png", "figure.svg"]);
    }

    /// Makes a file read-only, and writable again when dropped (also when the
    /// test panics), so that [`TempDir`] can remove it. Declare it after the
    /// [`TempDir`] so that it is dropped first.
    #[cfg(windows)]
    struct ReadOnly(PathBuf);

    #[cfg(windows)]
    impl ReadOnly {
        fn new(path: PathBuf) -> Self {
            Self::set(&path, true);
            ReadOnly(path)
        }

        fn set(path: &Path, readonly: bool) {
            let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
            permissions.set_readonly(readonly);
            std::fs::set_permissions(path, permissions).expect("set permissions");
        }
    }

    #[cfg(windows)]
    impl Drop for ReadOnly {
        fn drop(&mut self) {
            Self::set(&self.0, false);
        }
    }

    #[cfg(windows)]
    #[test]
    fn read_only_destination_is_an_error_that_leaves_no_temporary_file() {
        let dir = TempDir::new("read-only");
        let figure = one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]);
        let names = ["ro.pdf", "ro.png", "ro.svg"];
        let _guards: Vec<ReadOnly> = names
            .iter()
            .map(|name| {
                let path = dir.path(name);
                std::fs::write(&path, b"old").expect("write old file");
                ReadOnly::new(path)
            })
            .collect();
        for name in names {
            let path = dir.path(name);
            let err = write_figure(&path, &figure).expect_err("must fail");
            assert!(err.starts_with(&path.display().to_string()), "{err}");
            assert_eq!(std::fs::read(&path).unwrap(), b"old", "{name} changed");
        }
        assert_eq!(dir.entries(), names);
    }

    #[cfg(windows)]
    #[test]
    fn locked_destination_is_an_error_that_leaves_no_temporary_file() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new("locked");
        let path = dir.path("locked.png");
        std::fs::write(&path, b"old").expect("write old file");
        // Opened with no sharing, as some viewers keep a figure open.
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("open without sharing");
        let err = write_figure(&path, &one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]))
            .expect_err("must fail");
        drop(handle);
        assert!(err.starts_with(&path.display().to_string()), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(dir.entries(), ["locked.png"]);
    }

    #[cfg(windows)]
    #[test]
    fn ruviz_is_given_verbatim_paths() {
        for (temp, expected) in [
            (r"C:\a\b\.uvv-1-0.tmp", r"\\?\C:\a\b\.uvv-1-0.tmp"),
            (r"C:/a/../b/./.uvv-1-0.tmp", r"\\?\C:\b\.uvv-1-0.tmp"),
            (
                r"\\server\share\d\.uvv-1-0.tmp",
                r"\\?\UNC\server\share\d\.uvv-1-0.tmp",
            ),
            (r"\\?\C:\a\.uvv-1-0.tmp", r"\\?\C:\a\.uvv-1-0.tmp"),
        ] {
            assert_eq!(for_ruviz(Path::new(temp)).unwrap(), Path::new(expected));
        }
        let relative = for_ruviz(Path::new(".uvv-1-0.tmp")).unwrap();
        assert!(
            relative.to_str().unwrap().starts_with(r"\\?\"),
            "{relative:?}"
        );
        assert!(relative.ends_with(".uvv-1-0.tmp"), "{relative:?}");
    }

    /// Length of `path` in UTF-16 units, the unit of Windows' MAX_PATH.
    #[cfg(windows)]
    fn utf16_len(path: &Path) -> usize {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().count()
    }

    #[cfg(windows)]
    #[test]
    fn destination_close_to_max_path_is_written_without_temporary_files() {
        // ruviz moves its temporary file into place with MoveFileExW, which
        // fails for a path of MAX_PATH (260) UTF-16 units or more unless it is
        // a `\\?\` path. Handed these destinations directly, ruviz's temporary
        // `.{file name}.{pid}.{n}.tmp` is at most 20 units longer, so they
        // were written fine; a long and a short file name are both covered.
        const TOTALS: [usize; 2] = [230, 238];
        assert!(TOTALS.iter().all(|total| total + 20 < 260));
        let dir = TempDir::new("max-path");
        let figure = one_curve("a", vec![[1.0, 1.0], [2.0, 3.0]]);
        let names = [format!("{}.png", "n".repeat(40)), "a.png".to_string()];
        for (name, total) in names.iter().zip(TOTALS) {
            // `dir` + `\` + `d...d` + `\` + `name` makes `total` units.
            let pad = total - utf16_len(&dir.0) - 2 - name.len();
            let sub = dir.0.join("d".repeat(pad));
            let path = sub.join(name);
            assert_eq!(utf16_len(&path), total);
            write_figure(&path, &figure).expect("write figure");
            assert!(std::fs::read(&path).unwrap().starts_with(PNG_SIGNATURE));
            let entries: Vec<_> = std::fs::read_dir(&sub)
                .expect("read dir")
                .map(|e| e.expect("dir entry").file_name())
                .collect();
            assert_eq!(entries, [name.as_str()], "{total} units");
        }
    }
}
