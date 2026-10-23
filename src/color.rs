//! Curve colouring: gradient colormaps and a palette of distinct single colours.

use egui::Color32;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Colormap {
    Viridis,
    Jet,
    Coolwarm,
    BlueRed,
}

impl Colormap {
    pub const ALL: [Self; 4] = [Self::Viridis, Self::Jet, Self::Coolwarm, Self::BlueRed];

    pub fn name(self) -> &'static str {
        match self {
            Self::Viridis => "Viridis",
            Self::Jet => "Jet",
            Self::Coolwarm => "Coolwarm",
            Self::BlueRed => "Blue → Red",
        }
    }

    fn anchors(self) -> &'static [[u8; 3]] {
        match self {
            Self::Viridis => &[
                [68, 1, 84],
                [59, 82, 139],
                [33, 145, 140],
                [94, 201, 98],
                [253, 231, 37],
            ],
            Self::Jet => &[
                [0, 0, 143],
                [0, 0, 255],
                [0, 255, 255],
                [255, 255, 0],
                [255, 0, 0],
                [128, 0, 0],
            ],
            Self::Coolwarm => &[[59, 76, 192], [221, 221, 221], [180, 4, 38]],
            Self::BlueRed => &[[0, 60, 255], [255, 30, 30]],
        }
    }

    /// Colour at `t` in `0.0..=1.0`, linearly interpolated between anchors.
    pub fn sample(self, t: f32) -> Color32 {
        let anchors = self.anchors();
        let pos = t.clamp(0.0, 1.0) * (anchors.len() - 1) as f32;
        let i = (pos.floor() as usize).min(anchors.len() - 2);
        let f = pos - i as f32;
        let [a, b] = [anchors[i], anchors[i + 1]];
        let mix = |k: usize| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * f).round() as u8;
        Color32::from_rgb(mix(0), mix(1), mix(2))
    }
}

/// How a file's curves get their colour, unless a curve overrides it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColorMode {
    Gradient(Colormap),
    Single(Color32),
}

/// Distinct colours handed out to single-colour files by [`next_single`].
const PALETTE: [Color32; 8] = [
    Color32::from_rgb(31, 119, 180),
    Color32::from_rgb(214, 39, 40),
    Color32::from_rgb(44, 160, 44),
    Color32::from_rgb(255, 127, 14),
    Color32::from_rgb(148, 103, 189),
    Color32::from_rgb(140, 86, 75),
    Color32::from_rgb(227, 119, 194),
    Color32::from_rgb(23, 190, 207),
];

pub fn palette(index: usize) -> Color32 {
    PALETTE[index % PALETTE.len()]
}

/// Palette colour for a newly opened single-colour file, given the colours
/// the open files already use.
///
/// Picks the palette colour used the fewest times, earliest in the palette on
/// a tie: the first unused colour while any is left, and an even cycle once
/// all are taken.
pub fn next_single(in_use: impl IntoIterator<Item = Color32>) -> Color32 {
    let mut counts = [0usize; PALETTE.len()];
    for c in in_use {
        if let Some(i) = PALETTE.iter().position(|&p| p == c) {
            counts[i] += 1;
        }
    }
    let (i, _) = counts
        .iter()
        .enumerate()
        .min_by_key(|&(_, &n)| n)
        .expect("palette is not empty");
    PALETTE[i]
}

/// Where each curve sits on a gradient, in `0.0..=1.0`.
///
/// When every label is a number (the scan temperature in multi-column files)
/// the position follows that value, so e.g. the hottest scan always gets the
/// top of the colormap whether the run was heating or cooling. Otherwise the
/// curves are spread evenly in file order.
pub fn gradient_positions<'a>(labels: impl ExactSizeIterator<Item = &'a str>) -> Vec<f32> {
    let count = labels.len();
    let values: Option<Vec<f64>> = labels.map(|l| l.trim().parse::<f64>().ok()).collect();
    if let Some(values) = values {
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if max > min {
            return values
                .iter()
                .map(|v| ((v - min) / (max - min)) as f32)
                .collect();
        }
    }
    (0..count)
        .map(|i| {
            if count > 1 {
                i as f32 / (count - 1) as f32
            } else {
                0.0
            }
        })
        .collect()
}

/// Colour of a curve at gradient position `t` under `mode`.
pub fn curve_color(mode: ColorMode, t: f32) -> Color32 {
    match mode {
        ColorMode::Single(c) => c,
        ColorMode::Gradient(map) => map.sample(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_hits_both_ends() {
        let mode = ColorMode::Gradient(Colormap::BlueRed);
        assert_eq!(curve_color(mode, 0.0), Color32::from_rgb(0, 60, 255));
        assert_eq!(curve_color(mode, 1.0), Color32::from_rgb(255, 30, 30));
    }

    #[test]
    fn numeric_labels_position_by_value() {
        let t = gradient_positions(["110", "20", "65"].into_iter());
        assert_eq!(t, vec![1.0, 0.0, 0.5]);
    }

    #[test]
    fn text_labels_position_by_order() {
        let t = gradient_positions(["a", "b", "c"].into_iter());
        assert_eq!(t, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn next_single_takes_the_first_unused_colour() {
        assert_eq!(next_single([]), PALETTE[0]);
        assert_eq!(next_single([PALETTE[1], PALETTE[0]]), PALETTE[2]);
        // A gap left by a closed file is filled first.
        assert_eq!(next_single([PALETTE[1], PALETTE[2]]), PALETTE[0]);
        // Colours outside the palette (picked by hand) do not take a slot.
        assert_eq!(next_single([Color32::BLACK]), PALETTE[0]);
    }

    #[test]
    fn next_single_cycles_evenly_once_all_are_used() {
        let mut used = PALETTE.to_vec();
        assert_eq!(next_single(used.clone()), PALETTE[0]);
        used.push(PALETTE[0]);
        assert_eq!(next_single(used.clone()), PALETTE[1]);
        used.retain(|&c| c != PALETTE[5]);
        assert_eq!(next_single(used), PALETTE[5]);
    }

    #[test]
    fn sample_clamps_out_of_range() {
        for map in Colormap::ALL {
            assert_eq!(map.sample(-1.0), map.sample(0.0));
            assert_eq!(map.sample(2.0), map.sample(1.0));
        }
    }
}
