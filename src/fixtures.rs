//! Synthetic JASCO text exports used as test fixtures.
//!
//! The files in `testdata/` are produced by [`generate`] and committed, so the
//! tests never depend on real measurement data. Every value is invented: the
//! curves are built from rational functions and a small integer hash, using
//! only `+ - * /` (no `exp`/`sin`, whose last digits can vary between
//! platforms), so regenerating gives byte-identical files everywhere.
//!
//! The layout follows the real exports: a `KEY\tVALUE` header in the same key
//! order, `XYDATA`, the data, CRLF line endings and CP932 (Shift-JIS) encoding.
//!
//! - Regenerate: `cargo test -- --ignored regenerate_fixtures`
//! - `committed_fixtures_match_generator` fails when `testdata/` is stale.

use std::path::PathBuf;

/// Single column spectrum: descending equally spaced wavelengths, a few
/// values in exponent notation, then a blank line and an Extended section.
pub(crate) mod single {
    pub(crate) const FILE: &str = "single_spectrum.txt";
    /// Wavelength grid in tenths of a nanometre.
    const FIRST_X_TENTHS: i64 = 5600;
    const STEP_TENTHS: i64 = -4;
    pub(crate) const NPOINTS: usize = 651;

    pub(crate) fn x(i: usize) -> f64 {
        (FIRST_X_TENTHS + STEP_TENTHS * i as i64) as f64 / 10.0
    }

    pub(crate) fn y(i: usize) -> f64 {
        let x = x(i);
        super::band(x, 412.0, 12.0, 0.9) + super::band(x, 335.0, 20.0, 1.6) - 0.00012
            + 0.00002 * super::noise(1, i)
    }
}

/// Several scans side by side under a tab-led row of temperature labels.
pub(crate) mod multi {
    pub(crate) const FILE: &str = "multi_spectra.txt";
    pub(crate) const LABELS: [&str; 6] = ["85", "70.02", "55", "39.98", "25.01", "10"];
    const FIRST_X: i64 = 460;
    pub(crate) const NPOINTS: usize = 121;

    pub(crate) fn x(i: usize) -> f64 {
        (FIRST_X - i as i64) as f64
    }

    /// Absorbance of scan `k` at grid point `i`: a band near 395 nm that
    /// fades while one near 428 nm grows as the temperature falls.
    pub(crate) fn y(k: usize, i: usize) -> f64 {
        let x = x(i);
        let k = k as f64;
        super::band(x, 395.0, 15.0, 0.30 - 0.04 * k) + super::band(x, 428.0, 10.0, 0.05 * k) - 0.002
            + 0.0004 * k
            + 0.0005 * super::noise(2 + k as u64, i)
    }
}

/// Absorbance at one wavelength against an unevenly spaced temperature axis.
pub(crate) mod temperature {
    pub(crate) const FILE: &str = "temperature_scan.txt";
    /// Temperatures in hundredths of a degree.
    const FIRST_X_CENTI: i64 = 8500;
    const STEP_CENTI: i64 = -100;
    pub(crate) const NPOINTS: usize = 81;

    pub(crate) fn x(i: usize) -> f64 {
        // The real scans do not land exactly on the set points; the endpoints
        // stay exact so the header range is round.
        let jitter = if i == 0 || i + 1 == NPOINTS {
            0
        } else {
            (super::noise(9, i) * 4.0) as i64
        };
        (FIRST_X_CENTI + STEP_CENTI * i as i64 + jitter) as f64 / 100.0
    }

    pub(crate) fn y(i: usize) -> f64 {
        // Sigmoid-like transition around 45 C, built from r^12 / (1 + r^12).
        let r = 45.0 / x(i);
        let r2 = r * r;
        let r4 = r2 * r2;
        let r12 = r4 * r4 * r4;
        0.15 * r12 / (1.0 + r12) + 0.00001 * super::noise(10, i)
    }
}

/// A squared Lorentzian: peaks at `amplitude` on `center`, falls off quickly.
fn band(x: f64, center: f64, width: f64, amplitude: f64) -> f64 {
    let d = x - center;
    let l = width * width / (width * width + d * d);
    amplitude * l * l
}

/// Deterministic pseudo-noise in [-1, 1] from integer hashing only.
fn noise(seed: u64, i: usize) -> f64 {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64);
    s = s
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    s ^= s >> 29;
    s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    s ^= s >> 32;
    ((s % 2001) as f64 - 1000.0) / 1000.0
}

/// Formats like C's `%G` with six significant digits, as the exports do:
/// `0.00256498`, `1.90094`, `4.98927E-05`.
fn fmt_g(v: f64) -> String {
    let sci = format!("{v:.5e}");
    let (mantissa, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    if !(-4..6).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}E{sign}{:02}", trim_zeros(mantissa), exp.abs())
    } else {
        let decimals = (5 - exp) as usize;
        trim_zeros(&format!("{v:.decimals$}"))
    }
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s.to_owned()
    }
}

/// Header value for FIRSTX / LASTX.
fn fmt_header_x(v: f64) -> String {
    format!("{v:>10.4}")
}

/// Header value for FIRSTY / MAXY / MINY.
fn fmt_header_y(v: f64) -> String {
    format!("{v:>12.5}")
}

fn fmt_npoints(n: usize) -> String {
    format!("{n:>8}")
}

/// Joins lines with CRLF (including after the last one) and encodes as CP932.
fn encode(lines: &[String]) -> Vec<u8> {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push_str("\r\n");
    }
    let (bytes, _, had_errors) = encoding_rs::SHIFT_JIS.encode(&text);
    assert!(!had_errors, "fixture text is not representable in CP932");
    bytes.into_owned()
}

fn header(pairs: &[(&str, String)]) -> Vec<String> {
    pairs.iter().map(|(k, v)| format!("{k}\t{v}")).collect()
}

fn extremes(values: &[f64]) -> (f64, f64) {
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    (max, min)
}

fn single_spectrum() -> Vec<u8> {
    use single::*;
    let ys: Vec<f64> = (0..NPOINTS).map(y).collect();
    let (max, min) = extremes(&ys);
    let mut lines = header(&[
        ("TITLE", "Synthetic single spectrum".into()),
        ("DATA TYPE", "ULTRAVIOLET SPECTRUM".into()),
        ("ORIGIN", "JASCO".into()),
        ("OWNER", String::new()),
        ("DATE", "01/02/03".into()),
        ("TIME", "04:05:06".into()),
        ("SPECTROMETER/DATA SYSTEM", "Fixture generator".into()),
        ("LOCALE", "1041".into()),
        ("RESOLUTION", String::new()),
        ("DELTAX", "-0.4".into()),
        ("XUNITS", "NANOMETERS".into()),
        ("YUNITS", "ABSORBANCE".into()),
        ("FIRSTX", fmt_header_x(x(0))),
        ("LASTX", fmt_header_x(x(NPOINTS - 1))),
        ("NPOINTS", fmt_npoints(NPOINTS)),
        ("FIRSTY", fmt_header_y(ys[0])),
        ("MAXY", fmt_header_y(max)),
        ("MINY", fmt_header_y(min)),
    ]);
    lines.push("XYDATA".into());
    lines.extend((0..NPOINTS).map(|i| format!("{:.4}\t{}", x(i), fmt_g(ys[i]))));
    lines.push(String::new());
    lines.extend(
        [
            "##### Extended Information",
            "[コメント情報]",
            "試料名\t合成試料その一",
            "コメント\t単体テスト用の架空データ",
            "ユーザー名",
            "",
            "[データ情報]",
            "作成日時\t2001/02/03 04:05",
            "",
            "データタイプ\t等間隔データ",
            "横軸\tWavelength [nm]",
            "縦軸\tAbs",
            "スタート\t560 nm",
            "エンド\t300 nm",
            "データ間隔\t0.4 nm",
            "データ数\t651     ",
            "",
            "[測定情報]",
            "装置名\t架空分光器",
            "  温度\t21.50 ℃",
            "  セル\t石英 ±0 mm",
            "",
        ]
        .map(String::from),
    );
    encode(&lines)
}

fn multi_spectra() -> Vec<u8> {
    use multi::*;
    let ys: Vec<Vec<f64>> = (0..LABELS.len())
        .map(|k| (0..NPOINTS).map(|i| y(k, i)).collect())
        .collect();
    let all: Vec<f64> = ys.iter().flatten().copied().collect();
    let (max, min) = extremes(&all);
    let mut lines = header(&[
        ("TITLE", "Synthetic multi spectra".into()),
        ("DATA TYPE", "ULTRAVIOLET SPECTRUM".into()),
        ("ORIGIN", "JASCO".into()),
        ("OWNER", String::new()),
        ("DATE", "2002/03/04".into()),
        ("TIME", "05:06:07".into()),
        ("SPECTROMETER/DATA SYSTEM", String::new()),
        ("DELTAX", "-1".into()),
        ("XUNITS", "NANOMETERS".into()),
        ("YUNITS", "ABSORBANCE".into()),
        ("FIRSTX", fmt_header_x(x(0))),
        ("LASTX", fmt_header_x(x(NPOINTS - 1))),
        ("NPOINTS", fmt_npoints(NPOINTS)),
        ("FIRSTY", fmt_header_y(ys[0][0])),
        ("MAXY", fmt_header_y(max)),
        ("MINY", fmt_header_y(min)),
    ]);
    lines.push("XYDATA".into());
    lines.push(format!("\t{}", LABELS.join("\t")));
    for i in 0..NPOINTS {
        let mut line = fmt_g(x(i));
        for column in &ys {
            line.push('\t');
            line.push_str(&fmt_g(column[i]));
        }
        lines.push(line);
    }
    encode(&lines)
}

fn temperature_scan() -> Vec<u8> {
    use temperature::*;
    let mut lines = header(&[
        ("TITLE", "Synthetic temperature scan".into()),
        ("DATA TYPE", "ULTRAVIOLET SPECTRUM".into()),
        ("ORIGIN", "JASCO".into()),
        ("OWNER", String::new()),
        ("DATE", "03/04/05".into()),
        ("TIME", "06:07:08".into()),
        ("SPECTROMETER/DATA SYSTEM", String::new()),
        ("LOCALE", "1041".into()),
        ("XUNITS", "Temperature[C]".into()),
        ("YUNITS", "ABSORBANCE".into()),
        ("FIRSTX", fmt_header_x(x(0))),
        ("LASTX", fmt_header_x(x(NPOINTS - 1))),
        ("NPOINTS", fmt_npoints(NPOINTS)),
    ]);
    lines.push("XYDATA".into());
    lines.extend((0..NPOINTS).map(|i| format!("{:.4}\t{}", x(i), fmt_g(y(i)))));
    lines.push(String::new());
    lines.extend(
        [
            "##### Extended Information",
            "[コメント情報]",
            "試料名\t合成試料その二",
            "コメント",
            "",
            "[データ情報]",
            "作成日時\t2003/04/05 06:07",
            "",
            "データタイプ\t不等間隔データ",
            "横軸\tTemperature [C]",
            "縦軸\tAbs",
            "スタート\t85 C",
            "エンド\t5 C",
            "データ数\t81      ",
            "",
            "[測定情報]",
            "装置名\t架空分光器",
            "測定波長\t401 nm",
            "  開始モード\t設定温度 ±0.50 ℃に 3 秒間",
            "",
        ]
        .map(String::from),
    );
    encode(&lines)
}

/// Every fixture as `(file name, bytes)`.
pub(crate) fn generate() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (single::FILE, single_spectrum()),
        (multi::FILE, multi_spectra()),
        (temperature::FILE, temperature_scan()),
    ]
}

/// Directory holding the committed fixtures.
pub(crate) fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Reads a committed fixture. Read at run time rather than with
/// `include_bytes!`, so the crate still compiles when a fixture is missing
/// and `regenerate_fixtures` can recreate it.
pub(crate) fn read(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name))
        .unwrap_or_else(|e| panic!("testdata/{name}: {e}; run the regenerate_fixtures test"))
}

#[test]
#[ignore = "writes testdata/; run explicitly to regenerate the fixtures"]
fn regenerate_fixtures() {
    let dir = dir();
    std::fs::create_dir_all(&dir).expect("create testdata/");
    for (name, bytes) in generate() {
        std::fs::write(dir.join(name), bytes).expect("write fixture");
    }
}

#[test]
fn committed_fixtures_match_generator() {
    let dir = dir();
    let mut expected: Vec<&str> = Vec::new();
    for (name, bytes) in generate() {
        let committed = read(name);
        assert!(
            committed == bytes,
            "testdata/{name} differs from the generator; run \
             `cargo test -- --ignored regenerate_fixtures`"
        );
        expected.push(name);
    }
    let mut present: Vec<String> = std::fs::read_dir(&dir)
        .expect("read testdata/")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    present.sort();
    expected.sort();
    assert_eq!(
        present, expected,
        "testdata/ holds files the generator does not make"
    );
}
