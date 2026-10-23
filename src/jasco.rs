//! Parser for the text export of JASCO spectrometers (e.g. V-760).
//!
//! The files are CP932 (Shift-JIS) encoded. A block of `KEY\tVALUE` header
//! lines is followed by `XYDATA` and the data itself, in one of two layouts:
//!
//! - single column: `x\ty` per line;
//! - multi column: a tab-led label row (`\t110\t104.99\t...`, usually the
//!   temperature of each scan) followed by `x\ty1\ty2...` per line.
//!
//! Some files end with an `##### Extended Information` section, which is ignored.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Curve {
    pub label: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub name: String,
    pub path: PathBuf,
    pub x_unit: String,
    pub y_unit: String,
    pub curves: Vec<Curve>,
}

impl Dataset {
    /// True when the file held several scans side by side.
    pub fn is_multi(&self) -> bool {
        self.curves.len() > 1
    }
}

pub fn load(path: &Path) -> Result<Dataset, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut dataset = parse(&bytes, &name).map_err(|e| format!("{}: {e}", path.display()))?;
    dataset.path = path.to_path_buf();
    Ok(dataset)
}

pub fn parse(bytes: &[u8], name: &str) -> Result<Dataset, String> {
    let (text, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));

    let mut dataset = Dataset {
        name: name.to_owned(),
        ..Default::default()
    };

    let mut found_data = false;
    for line in lines.by_ref() {
        if line.trim() == "XYDATA" {
            found_data = true;
            break;
        }
        if let Some((key, value)) = line.split_once('\t') {
            match key.trim() {
                "XUNITS" => dataset.x_unit = value.trim().to_owned(),
                "YUNITS" => dataset.y_unit = value.trim().to_owned(),
                _ => {}
            }
        }
    }
    if !found_data {
        return Err("XYDATA section not found".to_owned());
    }

    let mut lines = lines.peekable();
    let labels: Vec<String> = match lines.peek() {
        Some(first) if first.starts_with('\t') => {
            let labels = first[1..]
                .split('\t')
                .map(|s| s.trim().to_owned())
                .collect();
            lines.next();
            labels
        }
        _ => vec![name.to_owned()],
    };

    let mut curves: Vec<Curve> = labels
        .into_iter()
        .map(|label| Curve {
            label,
            ..Default::default()
        })
        .collect();

    for line in lines {
        if line.trim().is_empty() || line.starts_with("#####") {
            break;
        }
        let mut cells = line.split('\t');
        let Some(x) = cells.next().and_then(|c| c.trim().parse::<f64>().ok()) else {
            continue;
        };
        for (curve, cell) in curves.iter_mut().zip(cells) {
            if let Ok(y) = cell.trim().parse::<f64>() {
                curve.x.push(x);
                curve.y.push(y);
            }
        }
    }

    curves.retain(|c| !c.x.is_empty());
    if curves.is_empty() {
        return Err("no data points".to_owned());
    }
    dataset.curves = curves;
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, cp932_header, lf_only, multi, single, temperature, time_course};

    /// The exports print six significant digits.
    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= expected.abs() * 1e-5,
            "{actual} != {expected}"
        );
    }

    /// The fixture's lines after `XYDATA`, decoded, without line endings.
    fn raw_lines_after_xydata(bytes: &[u8]) -> Vec<String> {
        let (text, _, had_errors) = encoding_rs::SHIFT_JIS.decode(bytes);
        assert!(!had_errors);
        text.lines()
            .map(|l| l.trim_end_matches('\r').to_owned())
            .skip_while(|l| l != "XYDATA")
            .skip(1)
            .collect()
    }

    #[test]
    fn parses_single_column_spectrum() {
        let bytes = fixtures::read(single::FILE);
        let ds = parse(&bytes, "single").unwrap();
        assert_eq!(ds.x_unit, "NANOMETERS");
        assert_eq!(ds.y_unit, "ABSORBANCE");
        assert_eq!(ds.curves.len(), 1);
        assert!(!ds.is_multi());
        let c = &ds.curves[0];
        assert_eq!(c.label, "single");
        assert_eq!(c.x.len(), single::NPOINTS);
        assert_eq!(c.x[0], single::x(0));
        assert_eq!(*c.x.last().unwrap(), single::x(single::NPOINTS - 1));
        // Written in exponent notation (`...E-05`).
        let first_row = &raw_lines_after_xydata(&bytes)[0];
        assert!(first_row.contains("E-"), "{first_row:?}");
        assert_close(c.y[0], single::y(0));
    }

    #[test]
    fn parses_multi_column_spectra() {
        let bytes = fixtures::read(multi::FILE);
        let ds = parse(&bytes, "multi").unwrap();
        assert!(ds.is_multi());
        assert_eq!(ds.curves.len(), multi::LABELS.len());
        assert_eq!(ds.curves[0].label, multi::LABELS[0]);
        assert_eq!(
            ds.curves.last().unwrap().label,
            *multi::LABELS.last().unwrap()
        );
        for c in &ds.curves {
            assert_eq!(c.x.len(), multi::NPOINTS);
        }
        assert_eq!(ds.curves[0].x[0], multi::x(0));
        assert_close(ds.curves[0].y[0], multi::y(0, 0));
    }

    #[test]
    fn parses_temperature_scan() {
        let bytes = fixtures::read(temperature::FILE);
        let ds = parse(&bytes, "temp").unwrap();
        assert_eq!(ds.x_unit, "Temperature[C]");
        assert_eq!(ds.curves.len(), 1);
        assert_eq!(ds.curves[0].x.len(), temperature::NPOINTS);
        assert_eq!(ds.curves[0].x[0], temperature::x(0));
        assert_close(ds.curves[0].y[0], temperature::y(0));
    }

    #[test]
    fn parses_time_course_in_transmittance() {
        let bytes = fixtures::read(time_course::FILE);
        let ds = parse(&bytes, "time").unwrap();
        assert_eq!(ds.x_unit, "SEC");
        assert_eq!(ds.y_unit, "TRANSMITTANCE");
        assert_eq!(ds.curves.len(), 1);
        let c = &ds.curves[0];
        assert_eq!(c.label, "time");
        assert_eq!(c.x.len(), time_course::NPOINTS);
        assert_eq!(c.x[0], time_course::x(0));
        let last = time_course::NPOINTS - 1;
        assert_eq!(c.x[last], time_course::x(last));
        assert_close(c.y[0], time_course::y(0));
        assert_close(c.y[last], time_course::y(last));
    }

    #[test]
    fn cp932_header_text_does_not_affect_units_or_data() {
        let bytes = fixtures::read(cp932_header::FILE);
        // Precondition: the header before XYDATA really holds CP932 bytes,
        // including a 0x5C trail byte (`表`, `ソ`).
        let xydata = bytes
            .windows(6)
            .position(|w| w == b"XYDATA")
            .expect("XYDATA");
        let (title, _, _) = encoding_rs::SHIFT_JIS.encode(cp932_header::TITLE);
        assert!(title.contains(&0x5C));
        assert!(bytes[..xydata].windows(title.len()).any(|w| w == &*title));

        let ds = parse(&bytes, "cp932").unwrap();
        assert_eq!(ds.x_unit, "NANOMETERS");
        assert_eq!(ds.y_unit, "ABSORBANCE");
        let labels: Vec<&str> = ds.curves.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, cp932_header::LABELS);
        for (k, c) in ds.curves.iter().enumerate() {
            assert_eq!(c.x[0], cp932_header::x(0));
            assert_close(c.y[0], cp932_header::y(k, 0));
        }
    }

    #[test]
    fn data_stops_at_extended_section() {
        let bytes = fixtures::read(cp932_header::FILE);
        // Precondition: `#####` follows the last data row directly, so only
        // the `#####` check (not the blank-line check) ends the data.
        let raw = raw_lines_after_xydata(&bytes);
        let last_row = cp932_header::NPOINTS; // after the label row
        assert!(raw[last_row].starts_with(&format!("{}\t", cp932_header::x(last_row - 1))));
        assert!(raw[last_row + 1].starts_with("#####"));

        let ds = parse(&bytes, "cp932").unwrap();
        let last = cp932_header::NPOINTS - 1;
        for c in &ds.curves {
            assert_eq!(c.x.len(), cp932_header::NPOINTS);
            assert_eq!(c.x[last], cp932_header::x(last));
            for x in cp932_header::EXTENDED_X {
                assert!(!c.x.contains(&x), "Extended row {x} read as data");
            }
            assert!(!c.y.contains(&cp932_header::EXTENDED_Y));
        }
    }

    #[test]
    fn lf_and_crlf_give_the_same_dataset() {
        let crlf = fixtures::read(single::FILE);
        let lf = fixtures::read(lf_only::FILE);
        assert!(crlf.windows(2).any(|w| w == b"\r\n"));
        assert!(!lf.contains(&b'\r'));

        let a = parse(&crlf, "same").unwrap();
        let b = parse(&lf, "same").unwrap();
        assert_eq!(a.x_unit, b.x_unit);
        assert_eq!(a.y_unit, b.y_unit);
        assert_eq!(a.curves.len(), b.curves.len());
        for (ca, cb) in a.curves.iter().zip(&b.curves) {
            assert_eq!(ca.label, cb.label);
            assert_eq!(ca.x, cb.x);
            assert_eq!(ca.y, cb.y);
        }
        assert_eq!(b.curves[0].x.len(), single::NPOINTS);
    }

    #[test]
    fn rejects_files_without_data() {
        assert!(parse(b"TITLE\tfoo\n", "x").is_err());
    }
}
