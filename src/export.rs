//! Writing the displayed curves to a CSV file.
//!
//! [`to_csv`] decides the content and is a pure function, so the format is
//! fixed by unit tests; [`write_csv`] only adds the byte order mark and writes.
//!
//! Layout, with two columns (x, y) per curve, in curve order:
//!
//! 1. the curve's name, in both its x and its y column;
//! 2. the x and y headers (axis names / units);
//! 3. and on: the `i`-th point of every curve. Each curve keeps its own x, so
//!    curves on different x grids stay paired with their own x values. A curve
//!    with fewer points than the longest one gets two empty cells per missing
//!    row.
//!
//! Fields are separated by `,` and every line, including the last, ends with
//! CRLF (RFC 4180). A header field containing `,`, `"`, CR or LF is enclosed in
//! double quotes, with `"` doubled. Numbers are written with `{}` formatting:
//! independent of the locale (the decimal mark is always `.`), never in
//! exponent notation, and parsing them back gives the original value exactly.
//! Non-finite values (NaN, ±inf) are written as empty cells.
//!
//! Names are written as given. No formula-injection escaping (such as a
//! leading `'`) is done: the names are the user's own file names, and that
//! escaping would corrupt legitimate names like `-5C_scan`.
//!
//! The file starts with a UTF-8 byte order mark (EF BB BF). Without it, Excel
//! on Japanese Windows reads the file as CP932 and Japanese file names come out
//! garbled. A `sep=,` first line is deliberately not written: with it, Excel
//! ignores the BOM.

use std::path::Path;

/// One curve as it is to be written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportCurve {
    pub name: String,
    pub x_header: String,
    pub y_header: String,
    pub points: Vec<[f64; 2]>,
}

/// UTF-8 byte order mark written before the CSV text.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Line terminator required by RFC 4180.
const EOL: &str = "\r\n";

/// Returns the CSV text (without byte order mark) for `curves`.
pub fn to_csv(curves: &[ExportCurve]) -> String {
    let mut out = String::new();
    push_line(
        &mut out,
        curves.iter().flat_map(|c| [quote(&c.name), quote(&c.name)]),
    );
    push_line(
        &mut out,
        curves
            .iter()
            .flat_map(|c| [quote(&c.x_header), quote(&c.y_header)]),
    );
    let rows = curves.iter().map(|c| c.points.len()).max().unwrap_or(0);
    for i in 0..rows {
        push_line(
            &mut out,
            curves.iter().flat_map(|c| match c.points.get(i) {
                Some(&[x, y]) => [number(x), number(y)],
                None => [String::new(), String::new()],
            }),
        );
    }
    out
}

/// Writes `curves` to `path` as a UTF-8 CSV file with a byte order mark.
///
/// On failure the error reads `{path}: {reason}`, like [`crate::jasco::load`].
pub fn write_csv(path: &Path, curves: &[ExportCurve]) -> Result<(), String> {
    let text = to_csv(curves);
    let mut bytes = Vec::with_capacity(BOM.len() + text.len());
    bytes.extend_from_slice(BOM);
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Appends `fields` joined by `,` and terminated by CRLF.
fn push_line(out: &mut String, fields: impl Iterator<Item = String>) {
    for (i, field) in fields.enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&field);
    }
    out.push_str(EOL);
}

/// A header field, enclosed in quotes only when RFC 4180 requires it.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// A numeric field: shortest round-trip decimal form, empty when not finite.
fn number(v: f64) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::fixtures;
    use crate::jasco;
    use crate::processing::{self, Baseline, Normalize};

    fn testdata(name: &str) -> PathBuf {
        fixtures::dir().join(name)
    }

    /// The data cells of a CSV line, for lines whose fields are not quoted.
    fn cells(line: &str) -> Vec<&str> {
        line.split(',').collect()
    }

    /// Splits `csv` into lines, checking that every line ends with CRLF.
    fn lines(csv: &str) -> Vec<&str> {
        let body = csv.strip_suffix("\r\n").expect("CSV text ends with CRLF");
        let lines: Vec<&str> = body.split("\r\n").collect();
        for line in &lines {
            assert!(!line.contains('\n') && !line.contains('\r'), "{line:?}");
        }
        lines
    }

    fn curve(name: &str, points: Vec<[f64; 2]>) -> ExportCurve {
        ExportCurve {
            name: name.into(),
            x_header: "x".into(),
            y_header: "y".into(),
            points,
        }
    }

    /// A temporary directory removed when dropped (also when the test panics).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("uv_viewer-export-{test}-{}", std::process::id()));
            // Clear a leftover from an earlier run that had the same PID.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Best-effort cleanup: a leftover temp directory must not fail the test.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Every curve of the single and the multi column fixture, processed, so
    /// the export covers two different x grids and point counts side by side.
    fn fixture_curves() -> Vec<ExportCurve> {
        let baseline = Baseline::RangeMean {
            lo: 340.0,
            hi: 360.0,
        };
        let normalize = Normalize::Max;
        let mut out = Vec::new();
        for file in [fixtures::single::FILE, fixtures::multi::FILE] {
            let dataset = jasco::load(&testdata(file)).expect("load fixture");
            for c in &dataset.curves {
                let name = if dataset.is_multi() {
                    format!("{} {}", dataset.name, c.label)
                } else {
                    dataset.name.clone()
                };
                out.push(ExportCurve {
                    name,
                    x_header: dataset.x_unit.clone(),
                    y_header: dataset.y_unit.clone(),
                    points: processing::apply(c, &baseline, &normalize),
                });
            }
        }
        out
    }

    #[test]
    fn fixture_curves_round_trip_exactly() {
        let curves = fixture_curves();
        assert_eq!(curves.len(), 1 + fixtures::multi::LABELS.len());
        let csv = to_csv(&curves);
        let lines = lines(&csv);
        let longest = fixtures::single::NPOINTS;
        assert_eq!(lines.len(), 2 + longest);

        let names = cells(lines[0]);
        let headers = cells(lines[1]);
        for (k, c) in curves.iter().enumerate() {
            assert_eq!(names[2 * k], c.name);
            assert_eq!(names[2 * k + 1], c.name);
            assert_eq!(headers[2 * k], c.x_header);
            assert_eq!(headers[2 * k + 1], c.y_header);
        }
        assert_eq!(names[0], "single_spectrum");
        assert_eq!(names[2], "multi_spectra 85");
        assert_eq!(headers[0], "NANOMETERS");
        assert_eq!(headers[1], "ABSORBANCE");

        for (i, line) in lines[2..].iter().enumerate() {
            let row = cells(line);
            assert_eq!(row.len(), 2 * curves.len(), "row {i}");
            for (k, c) in curves.iter().enumerate() {
                match c.points.get(i) {
                    Some([x, y]) => {
                        let got_x: f64 = row[2 * k].parse().expect("x parses");
                        let got_y: f64 = row[2 * k + 1].parse().expect("y parses");
                        assert_eq!(got_x.to_bits(), x.to_bits(), "row {i} curve {k} x");
                        assert_eq!(got_y.to_bits(), y.to_bits(), "row {i} curve {k} y");
                    }
                    None => {
                        assert_eq!(row[2 * k], "", "row {i} curve {k} x");
                        assert_eq!(row[2 * k + 1], "", "row {i} curve {k} y");
                    }
                }
            }
        }
        // The processing must actually have changed the values, or this test
        // would not show that processed (not raw) values are written.
        let raw = jasco::load(&testdata(fixtures::single::FILE)).expect("load fixture");
        assert_ne!(curves[0].points[0][1], raw.curves[0].y[0]);
    }

    #[test]
    fn short_curves_are_padded_with_empty_cells() {
        let curves = [
            curve("a", vec![[1.0, 10.0]]),
            curve("b", vec![[2.0, 20.0], [3.0, 30.0], [4.0, 40.0]]),
            curve("c", vec![]),
        ];
        let csv = to_csv(&curves);
        assert_eq!(
            csv,
            "a,a,b,b,c,c\r\n\
             x,y,x,y,x,y\r\n\
             1,10,2,20,,\r\n\
             ,,3,30,,\r\n\
             ,,4,40,,\r\n"
        );
        let lines = lines(&csv);
        assert_eq!(lines.len(), 2 + 3);
        for line in &lines {
            assert_eq!(cells(line).len(), 2 * curves.len(), "{line:?}");
        }
    }

    #[test]
    fn numbers_use_plain_decimal_notation() {
        let csv = to_csv(&[curve("n", vec![[1e-7, -2.5e21], [0.1, 300.0]])]);
        assert_eq!(
            csv,
            "n,n\r\nx,y\r\n0.0000001,-2500000000000000000000\r\n0.1,300\r\n"
        );
    }

    #[test]
    fn headers_with_separators_or_quotes_are_quoted() {
        let mut c = curve("a,b", vec![]);
        c.x_header = "say \"hi\"".into();
        c.y_header = "two\r\nlines".into();
        let plain = curve("-5C_scan", vec![]);
        let lf = ExportCurve {
            name: "cr\rlf\n".into(),
            ..curve("", vec![])
        };
        let csv = to_csv(&[c, plain, lf]);
        assert_eq!(
            csv,
            "\"a,b\",\"a,b\",-5C_scan,-5C_scan,\"cr\rlf\n\",\"cr\rlf\n\"\r\n\
             \"say \"\"hi\"\"\",\"two\r\nlines\",x,y,x,y\r\n"
        );
    }

    #[test]
    fn non_finite_values_are_empty_cells() {
        let csv = to_csv(&[curve(
            "n",
            vec![
                [f64::NAN, 1.0],
                [2.0, f64::INFINITY],
                [f64::NEG_INFINITY, f64::NAN],
            ],
        )]);
        assert_eq!(csv, "n,n\r\nx,y\r\n,1\r\n2,\r\n,\r\n");
    }

    #[test]
    fn write_csv_prepends_bom() {
        let tmp = TempDir::new("write_csv_prepends_bom");
        let path = tmp.0.join("out.csv");
        let curves = [curve("日本語の名前", vec![[1.0, 2.0]])];
        write_csv(&path, &curves).expect("write");
        let bytes = std::fs::read(&path).expect("read back");
        let rest = bytes
            .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
            .expect("starts with UTF-8 BOM");
        assert_eq!(rest, to_csv(&curves).as_bytes());
    }

    #[test]
    fn write_csv_reports_failure() {
        let tmp = TempDir::new("write_csv_reports_failure");
        let path = tmp.0.join("missing").join("out.csv");
        let err = write_csv(&path, &[curve("a", vec![[1.0, 2.0]])]).expect_err("must fail");
        assert!(err.starts_with(&format!("{}: ", path.display())), "{err:?}");
        assert!(!path.exists());
    }
}
