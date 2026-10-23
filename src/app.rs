//! The application state and its per-frame update logic.

use std::path::PathBuf;

use egui::{Color32, RichText};
use egui_plot::{Legend, Line, Plot};
use serde::{Deserialize, Serialize};

use crate::color::{self, ColorMode, Colormap};
use crate::jasco::{self, Dataset};
use crate::processing::{self, Baseline, Normalize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum BaselineKind {
    None,
    RangeMean,
    TwoPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum NormalizeKind {
    None,
    Max,
    AtX,
}

/// Processing settings shared by every curve.
///
/// The parameters of each mode are kept even while another mode is selected,
/// so switching back and forth does not lose them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Processing {
    baseline_kind: BaselineKind,
    range: [f64; 2],
    points: [f64; 2],
    normalize_kind: NormalizeKind,
    norm_x: f64,
}

impl Default for Processing {
    fn default() -> Self {
        Self {
            baseline_kind: BaselineKind::None,
            range: [480.0, 500.0],
            points: [400.0, 500.0],
            normalize_kind: NormalizeKind::None,
            norm_x: 350.0,
        }
    }
}

impl Processing {
    fn baseline(&self) -> Baseline {
        match self.baseline_kind {
            BaselineKind::None => Baseline::None,
            BaselineKind::RangeMean => Baseline::RangeMean {
                lo: self.range[0],
                hi: self.range[1],
            },
            BaselineKind::TwoPoint => Baseline::TwoPoint {
                x1: self.points[0],
                x2: self.points[1],
            },
        }
    }

    fn normalize(&self) -> Normalize {
        match self.normalize_kind {
            NormalizeKind::None => Normalize::None,
            NormalizeKind::Max => Normalize::Max,
            NormalizeKind::AtX => Normalize::AtX(self.norm_x),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CurveState {
    visible: bool,
    /// Manually picked colour; `None` follows the file's [`ColorMode`].
    color: Option<Color32>,
}

impl Default for CurveState {
    fn default() -> Self {
        Self {
            visible: true,
            color: None,
        }
    }
}

/// One opened file. Only the path and display settings are persisted; the
/// data is re-read from disk on startup.
#[derive(Serialize, Deserialize)]
struct LoadedFile {
    path: PathBuf,
    visible: bool,
    color_mode: ColorMode,
    curves: Vec<CurveState>,

    #[serde(skip)]
    dataset: Dataset,
    /// Gradient position of each curve, derived from the dataset.
    #[serde(skip)]
    gradient: Vec<f32>,
}

impl LoadedFile {
    /// `single_color` is the colour a single-curve file starts with.
    fn new(dataset: Dataset, single_color: Color32) -> Self {
        let color_mode = if dataset.is_multi() {
            ColorMode::Gradient(Colormap::Coolwarm)
        } else {
            ColorMode::Single(single_color)
        };
        let mut file = Self {
            path: dataset.path.clone(),
            visible: true,
            color_mode,
            curves: Vec::new(),
            dataset: Dataset::default(),
            gradient: Vec::new(),
        };
        file.attach(dataset);
        file
    }

    /// Installs freshly loaded data, keeping the curve settings when they
    /// still match the file's layout.
    fn attach(&mut self, dataset: Dataset) {
        if self.curves.len() != dataset.curves.len() {
            self.curves = vec![CurveState::default(); dataset.curves.len()];
        }
        self.gradient = color::gradient_positions(dataset.curves.iter().map(|c| c.label.as_str()));
        self.dataset = dataset;
    }

    fn curve_color(&self, index: usize) -> Color32 {
        self.curves[index]
            .color
            .unwrap_or_else(|| color::curve_color(self.color_mode, self.gradient[index]))
    }
}

/// Drag-and-drop payload for reordering the file list: the dragged file's index.
struct FileDrag(usize);

/// Persisted application state.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct UvViewerApp {
    files: Vec<LoadedFile>,
    processing: Processing,
    show_legend: bool,

    #[serde(skip)]
    errors: Vec<String>,
    #[serde(skip)]
    show_about: bool,
}

impl Default for UvViewerApp {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            processing: Processing::default(),
            show_legend: true,
            errors: Vec::new(),
            show_about: false,
        }
    }
}

/// Enlarges the whole UI by 10% on top of the OS display scaling.
fn apply_ui_zoom(ctx: &egui::Context) {
    ctx.set_zoom_factor(1.1);
}

impl UvViewerApp {
    /// Builds the app, restoring persisted state when eframe has some.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_ui_zoom(&cc.egui_ctx);

        let mut app: Self = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY))
            .unwrap_or_default();
        app.reload_files();
        app
    }

    /// Re-reads every persisted file from disk, dropping the ones that are gone.
    fn reload_files(&mut self) {
        let mut errors = Vec::new();
        self.files.retain_mut(|file| match jasco::load(&file.path) {
            Ok(dataset) => {
                file.attach(dataset);
                true
            }
            Err(e) => {
                errors.push(e);
                false
            }
        });
        self.errors = errors;
    }

    fn open_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.files.iter().any(|f| f.path == path) {
                continue;
            }
            match jasco::load(&path) {
                Ok(dataset) => {
                    // Avoid the colours the open files already show, rather than
                    // going by list position, which repeats after a close or move.
                    // Per-curve overrides are deliberate picks and do not count.
                    let in_use = self.files.iter().filter_map(|f| match f.color_mode {
                        ColorMode::Single(c) => Some(c),
                        ColorMode::Gradient(_) => None,
                    });
                    let single_color = color::next_single(in_use);
                    self.files.push(LoadedFile::new(dataset, single_color));
                }
                Err(e) => self.errors.push(e),
            }
        }
    }

    fn open_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("JASCO text", &["txt"])
            .add_filter("All files", &["*"])
            .pick_files()
        {
            self.open_paths(paths);
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_paths(dropped);
        }
    }

    fn processing_ui(&mut self, ui: &mut egui::Ui) {
        let p = &mut self.processing;

        ui.label(RichText::new("Baseline").strong());
        ui.radio_value(&mut p.baseline_kind, BaselineKind::None, "None");
        ui.horizontal(|ui| {
            ui.radio_value(&mut p.baseline_kind, BaselineKind::RangeMean, "Range mean");
            ui.add_enabled_ui(p.baseline_kind == BaselineKind::RangeMean, |ui| {
                x_value(ui, &mut p.range[0]);
                ui.label("–");
                x_value(ui, &mut p.range[1]);
            });
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut p.baseline_kind, BaselineKind::TwoPoint, "Two-point");
            ui.add_enabled_ui(p.baseline_kind == BaselineKind::TwoPoint, |ui| {
                x_value(ui, &mut p.points[0]);
                ui.label("/");
                x_value(ui, &mut p.points[1]);
            });
        });

        ui.add_space(6.0);
        ui.label(RichText::new("Normalize").strong());
        ui.radio_value(&mut p.normalize_kind, NormalizeKind::None, "None");
        ui.radio_value(&mut p.normalize_kind, NormalizeKind::Max, "Max = 1");
        ui.horizontal(|ui| {
            ui.radio_value(&mut p.normalize_kind, NormalizeKind::AtX, "Value at x = 1");
            ui.add_enabled_ui(p.normalize_kind == NormalizeKind::AtX, |ui| {
                x_value(ui, &mut p.norm_x);
            });
        });
    }

    /// Moves the file at `from` so that it lands before the file currently at
    /// `to` (`to == len` moves it to the end).
    fn move_file(&mut self, from: usize, to: usize) {
        let file = self.files.remove(from);
        self.files.insert(if to > from { to - 1 } else { to }, file);
    }

    fn files_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let dragged = egui::DragAndDrop::payload::<FileDrag>(ui.ctx()).map(|p| p.0);
        let pointer = ui.ctx().pointer_interact_pos();
        let mut drop_to = None;
        for (fi, file) in self.files.iter_mut().enumerate() {
            let id = ui.make_persistent_id(("file", &file.path));
            let rect = ui
                .scope(|ui| {
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        id,
                        false,
                    )
                    .show_header(ui, |ui| {
                        ui.add(
                            egui::Label::new("☰")
                                .selectable(false)
                                .sense(egui::Sense::drag()),
                        )
                        .on_hover_text("Drag to reorder")
                        .dnd_set_drag_payload(FileDrag(fi));
                        ui.checkbox(&mut file.visible, "");
                        ui.label(&file.dataset.name)
                            .on_hover_text(file.path.display().to_string());
                        if ui.small_button("✖").on_hover_text("Close file").clicked() {
                            remove = Some(fi);
                        }
                    })
                    .body(|ui| file_body_ui(ui, file));
                })
                .response
                .rect;

            // While dragging, mark the drop position above or below this file.
            if dragged.is_some()
                && let Some(pos) = pointer
            {
                let rect = rect.expand2(egui::vec2(0.0, ui.spacing().item_spacing.y / 2.0));
                if rect.contains(pos) {
                    let below = pos.y > rect.center().y;
                    let y = if below { rect.bottom() } else { rect.top() };
                    ui.painter()
                        .hline(rect.x_range(), y, ui.visuals().selection.stroke);
                    drop_to = Some(fi + usize::from(below));
                }
            }
        }
        if let Some(i) = remove {
            self.files.remove(i);
        } else if let (Some(from), Some(to)) = (dragged, drop_to)
            && ui.input(|i| i.pointer.any_released())
        {
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.move_file(from, to);
        }
    }

    fn plot_ui(&self, ui: &mut egui::Ui) {
        if self.files.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("File → Open… or drop JASCO .txt files here");
            });
            return;
        }

        let baseline = self.processing.baseline();
        let normalize = self.processing.normalize();
        let first = self
            .files
            .iter()
            .find(|f| f.visible)
            .unwrap_or(&self.files[0]);
        let x_label = axis_label(&first.dataset.x_unit);
        let y_label = if normalize == Normalize::None {
            axis_label(&first.dataset.y_unit)
        } else {
            "Normalized".to_owned()
        };

        let mut plot = Plot::new("spectra")
            .x_axis_label(x_label)
            .y_axis_label(y_label);
        if self.show_legend {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot_ui| {
            for file in self.files.iter().filter(|f| f.visible) {
                for (ci, curve) in file.dataset.curves.iter().enumerate() {
                    if !file.curves[ci].visible {
                        continue;
                    }
                    let name = if file.dataset.is_multi() {
                        format!("{}: {}", file.dataset.name, curve.label)
                    } else {
                        file.dataset.name.clone()
                    };
                    let points = processing::apply(curve, &baseline, &normalize);
                    plot_ui.line(
                        Line::new(name, points)
                            // Keyed by path so legend-hidden curves survive a reorder.
                            .id(egui::Id::new(("curve", &file.path, ci)))
                            .color(file.curve_color(ci))
                            .width(1.5),
                    );
                }
            }
        });
    }
}

fn file_body_ui(ui: &mut egui::Ui, file: &mut LoadedFile) {
    ui.horizontal(|ui| {
        let gradient = matches!(file.color_mode, ColorMode::Gradient(_));
        if ui.selectable_label(gradient, "Gradient").clicked() && !gradient {
            file.color_mode = ColorMode::Gradient(Colormap::Coolwarm);
        }
        if ui.selectable_label(!gradient, "Single").clicked() && gradient {
            file.color_mode = ColorMode::Single(file.curve_color(0));
        }
        match &mut file.color_mode {
            ColorMode::Gradient(map) => {
                egui::ComboBox::from_id_salt(("colormap", &file.path))
                    .selected_text(map.name())
                    .show_ui(ui, |ui| {
                        for m in Colormap::ALL {
                            ui.selectable_value(map, m, m.name());
                        }
                    });
            }
            ColorMode::Single(c) => {
                ui.color_edit_button_srgba(c);
            }
        }
    });

    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            file.curves.iter_mut().for_each(|c| c.visible = true);
        }
        if ui.small_button("Hide all").clicked() {
            file.curves.iter_mut().for_each(|c| c.visible = false);
        }
        if file.curves.iter().any(|c| c.color.is_some())
            && ui.small_button("Reset colors").clicked()
        {
            file.curves.iter_mut().for_each(|c| c.color = None);
        }
    });

    for ci in 0..file.curves.len() {
        let mut color = file.curve_color(ci);
        let label = file.dataset.curves[ci].label.clone();
        let state = &mut file.curves[ci];
        ui.horizontal(|ui| {
            if ui.color_edit_button_srgba(&mut color).changed() {
                state.color = Some(color);
            }
            ui.checkbox(&mut state.visible, label);
            if state.color.is_some() && ui.small_button("↺").on_hover_text("Reset color").clicked()
            {
                state.color = None;
            }
        });
    }
}

/// A compact drag box for an x-axis position (wavelength or temperature).
fn x_value(ui: &mut egui::Ui, value: &mut f64) {
    ui.add(egui::DragValue::new(value).speed(0.5).max_decimals(2));
}

/// Turns the JASCO unit strings into axis titles.
fn axis_label(unit: &str) -> String {
    match unit {
        "NANOMETERS" => "Wavelength / nm".to_owned(),
        "ABSORBANCE" => "Absorbance".to_owned(),
        u if u.starts_with("Temperature") => "Temperature / °C".to_owned(),
        u => u.to_owned(),
    }
}

impl eframe::App for UvViewerApp {
    /// Called by eframe on a timer and at shutdown (needs the `persistence` feature).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// egui 0.35 hands the app a root [`egui::Ui`] rather than a `Context`;
    /// panels are nested inside it.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Cloning a Context is cheap (it is an Arc) and frees up `ui` for the
        // panels below, since `egui::Window` still wants a `&Context`.
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        ui.close();
                        self.open_dialog();
                    }
                    if ui.button("Close all").clicked() {
                        self.files.clear();
                        ui.close();
                    }
                    ui.separator();
                    // Quitting from a menu is desktop-only; on web there is no window to close.
                    if !cfg!(target_arch = "wasm32") && ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close();
                    }
                });

                ui.add_space(16.0);
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        egui::Panel::left("side_panel")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Processing");
                    self.processing_ui(ui);

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.heading("Files");
                        if ui.button("Open…").clicked() {
                            self.open_dialog();
                        }
                    });
                    ui.checkbox(&mut self.show_legend, "Show legend");
                    self.files_ui(ui);

                    if !self.errors.is_empty() {
                        ui.separator();
                        for e in &self.errors {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        if ui.small_button("Dismiss").clicked() {
                            self.errors.clear();
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| self.plot_ui(ui));

        if self.show_about {
            let mut open = self.show_about;
            egui::Window::new("About")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&ctx, |ui| {
                    ui.heading(crate::APP_NAME);
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.separator();
                    ui.label("Viewer for JASCO UV-Vis text exports.");
                });
            self.show_about = open;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use std::path::Path;

    fn testdata(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name)
    }

    /// The 1.1x zoom must be relative to the OS scaling, not an absolute pixel
    /// scale that shrinks the UI on HiDPI displays.
    #[test]
    fn ui_zoom_is_relative_to_native_scaling() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .native_pixels_per_point = Some(1.5);

        // eframe has already fed the native scale in before the app is built.
        let _ = ctx.run_ui(input.clone(), |_| {});
        apply_ui_zoom(&ctx);
        let _ = ctx.run_ui(input, |_| {});

        assert_eq!(ctx.zoom_factor(), 1.1);
        assert!((ctx.pixels_per_point() - 1.65).abs() < 1e-6);
    }

    #[test]
    fn opening_the_same_file_twice_is_ignored() {
        let mut app = UvViewerApp::default();
        let path = testdata(fixtures::multi::FILE);
        app.open_paths([path.clone(), path]);
        assert_eq!(app.files.len(), 1);
        assert_eq!(app.files[0].curves.len(), fixtures::multi::LABELS.len());
        assert!(matches!(app.files[0].color_mode, ColorMode::Gradient(_)));
    }

    #[test]
    fn move_file_reorders_without_touching_colors() {
        let names = [fixtures::multi::FILE, fixtures::single::FILE];
        let mut app = UvViewerApp::default();
        app.open_paths(names.map(testdata));
        let order =
            |app: &UvViewerApp| app.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        let colors = |app: &UvViewerApp| {
            app.files
                .iter()
                .map(|f| (f.path.clone(), f.curve_color(0)))
                .collect::<Vec<_>>()
        };
        let before = colors(&app);

        app.move_file(0, 2);
        assert_eq!(order(&app), [testdata(names[1]), testdata(names[0])]);
        app.move_file(1, 0);
        assert_eq!(order(&app), [testdata(names[0]), testdata(names[1])]);
        // Dropping a file right above or below itself is a no-op.
        app.move_file(0, 0);
        app.move_file(0, 1);
        assert_eq!(colors(&app), before);
    }

    /// Copies the single-curve fixture to `count` distinct paths in a per-test
    /// temporary directory, since opening the same path twice is ignored.
    fn single_curve_copies(test: &str, count: usize) -> Vec<PathBuf> {
        let dir = std::env::temp_dir().join(format!("uv_viewer-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        (0..count)
            .map(|i| {
                let path = dir.join(format!("single_{i}.txt"));
                std::fs::copy(testdata(fixtures::single::FILE), &path).expect("copy fixture");
                path
            })
            .collect()
    }

    fn single_colors(app: &UvViewerApp) -> Vec<Color32> {
        app.files
            .iter()
            .map(|f| match f.color_mode {
                ColorMode::Single(c) => c,
                ColorMode::Gradient(_) => panic!("expected a single-curve file"),
            })
            .collect()
    }

    #[test]
    fn closing_a_file_does_not_hand_out_a_colour_in_use() {
        let paths = single_curve_copies("close", 3);
        let mut app = UvViewerApp::default();
        app.open_paths(paths[..2].to_vec());
        app.files.remove(0);
        app.open_paths([paths[2].clone()]);

        let colors = single_colors(&app);
        assert_eq!(colors.len(), 2);
        assert_ne!(colors[0], colors[1]);
    }

    #[test]
    fn reordering_files_does_not_hand_out_a_colour_in_use() {
        let paths = single_curve_copies("reorder", 3);
        let mut app = UvViewerApp::default();
        app.open_paths(paths[..2].to_vec());
        app.move_file(1, 0);
        app.files.remove(1);
        app.open_paths([paths[2].clone()]);

        let colors = single_colors(&app);
        assert_eq!(colors.len(), 2);
        assert_ne!(colors[0], colors[1]);
    }

    #[test]
    fn missing_files_are_reported() {
        let mut app = UvViewerApp::default();
        app.open_paths([testdata("does_not_exist.txt")]);
        assert!(app.files.is_empty());
        assert_eq!(app.errors.len(), 1);
    }

    /// Guards the persistence contract: settings survive a serialize round-trip
    /// and the data is re-read from the stored paths.
    #[test]
    fn state_survives_a_serde_round_trip() {
        let mut app = UvViewerApp::default();
        app.open_paths([
            testdata(fixtures::multi::FILE),
            testdata(fixtures::single::FILE),
        ]);
        app.processing.baseline_kind = BaselineKind::TwoPoint;
        app.files[0].curves[3].visible = false;
        app.files[0].curves[4].color = Some(Color32::RED);

        let json = serde_json::to_string(&app).expect("serialize");
        let mut restored: UvViewerApp = serde_json::from_str(&json).expect("deserialize");
        restored.reload_files();

        assert_eq!(restored.processing, app.processing);
        assert_eq!(restored.files.len(), 2);
        assert_eq!(
            restored.files[0].dataset.curves.len(),
            fixtures::multi::LABELS.len()
        );
        assert!(!restored.files[0].curves[3].visible);
        assert_eq!(restored.files[0].curve_color(4), Color32::RED);
        assert!(restored.errors.is_empty());
    }
}
