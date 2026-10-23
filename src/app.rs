//! The application state and its per-frame update logic.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use egui::{Color32, RichText};
use egui_plot::{Legend, Line, Plot};
use serde::{Deserialize, Serialize};

use crate::color::{self, ColorMode, Colormap};
use crate::export;
use crate::figure;
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

    /// Like [`Self::new`], then opens the files named on the command line
    /// (e.g. from "Open with", a file dropped on the executable, or the
    /// desktop entry's `%F`). `args` excludes the program name.
    pub fn with_args(
        cc: &eframe::CreationContext<'_>,
        args: impl IntoIterator<Item = OsString>,
    ) -> Self {
        let mut app = Self::new(cc);
        app.open_startup_args(args);
        app
    }

    /// Opens command-line arguments on top of the restored state.
    ///
    /// The app takes no options, so arguments starting with `-` are skipped
    /// instead of being reported as missing files: old macOS Finder passes
    /// `-psn_...`, and launchers pass absolute paths, which never start with
    /// `-` (a file named like that can still be given as `./-name`).
    fn open_startup_args(&mut self, args: impl IntoIterator<Item = OsString>) {
        let paths = args
            .into_iter()
            .filter(|arg| !arg.as_encoded_bytes().starts_with(b"-"))
            .map(|arg| {
                let path = PathBuf::from(arg);
                // Persisted paths are reloaded next session, possibly from
                // another working directory. `absolute` only fails for an empty
                // path or an unreadable working directory; then the path is kept
                // as given and loading it reports the problem.
                std::path::absolute(&path).unwrap_or(path)
            });
        self.open_paths(paths);
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
                    // Once all palette colours are taken, they are reused evenly.
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

        let Some((x_label, y_label)) = self.axis_titles() else {
            return;
        };

        let mut plot = Plot::new("spectra")
            .x_axis_label(x_label)
            .y_axis_label(y_label);
        if self.show_legend {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot_ui| {
            for curve in self.visible_curves() {
                plot_ui.line(
                    Line::new(curve.name, curve.points)
                        // Keyed by path so legend-hidden curves survive a reorder.
                        .id(egui::Id::new(("curve", curve.path, curve.index)))
                        .color(curve.color)
                        .width(1.5),
                );
            }
        });
    }

    /// The plot's x and y axis titles, taken from the first visible file (or
    /// the first file when none is visible); `None` when no file is open.
    fn axis_titles(&self) -> Option<(String, String)> {
        let first = self
            .files
            .iter()
            .find(|f| f.visible)
            .or(self.files.first())?;
        Some(axis_titles(&first.dataset, self.processing.normalize()))
    }

    /// The shown curves as they are written to CSV, headed like the plot axes.
    fn export_curves(&self) -> Vec<export::ExportCurve> {
        let normalize = self.processing.normalize();
        self.visible_curves()
            .into_iter()
            .map(|curve| {
                let (x_header, y_header) = axis_titles(curve.dataset, normalize);
                export::ExportCurve {
                    name: curve.name,
                    x_header,
                    y_header,
                    points: curve.points,
                }
            })
            .collect()
    }

    /// Writes the shown curves to `path`; failures go to the errors list.
    ///
    /// With no curve shown nothing is written and that is reported instead:
    /// the menu item is disabled then, and an export with no columns would
    /// only be two empty lines.
    fn export_to(&mut self, path: &Path) {
        let curves = self.export_curves();
        if curves.is_empty() {
            self.errors.push(format!(
                "{}: nothing to export, no curve is shown",
                path.display()
            ));
            return;
        }
        if let Err(e) = export::write_csv(path, &curves) {
            self.errors.push(e);
        }
    }

    /// Where the Export dialog starts: the folder of the first shown curve's
    /// file, and "<that file's name>_export.csv". `None` when nothing is shown.
    fn export_default_location(&self) -> Option<(PathBuf, String)> {
        self.save_default_location("_export.csv")
    }

    /// The folder of the first shown curve's file, and that file's name
    /// followed by `suffix`. `None` when nothing is shown.
    fn save_default_location(&self, suffix: &str) -> Option<(PathBuf, String)> {
        let curves = self.visible_curves();
        let first = curves.first()?;
        let dir = first.path.parent()?.to_path_buf();
        Some((dir, format!("{}{suffix}", first.dataset.name)))
    }

    fn export_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("CSV", &["csv"]);
        if let Some((dir, file_name)) = self.export_default_location() {
            dialog = dialog.set_directory(dir).set_file_name(file_name);
        }
        if let Some(path) = dialog.save_file() {
            self.export_to(&path);
        }
    }

    /// Whether any curve is shown, i.e. whether [`Self::visible_curves`] is
    /// non-empty, without processing the curves.
    fn any_curve_shown(&self) -> bool {
        self.files
            .iter()
            .any(|f| f.visible && f.curves.iter().any(|c| c.visible))
    }

    /// The shown curves as the plot draws them (processed values, colours,
    /// legend names, axis titles and legend setting). `None` when no curve is
    /// shown.
    fn figure(&self) -> Option<figure::Figure> {
        let (x_title, y_title) = self.axis_titles()?;
        let curves: Vec<figure::FigureCurve> = self
            .visible_curves()
            .into_iter()
            .map(|curve| figure::FigureCurve {
                name: curve.name,
                points: curve.points,
                color: curve.color,
            })
            .collect();
        if curves.is_empty() {
            return None;
        }
        Some(figure::Figure {
            curves,
            x_title,
            y_title,
            legend: self.show_legend,
        })
    }

    /// Draws the shown curves to `path` (format by its extension); failures
    /// go to the errors list. Runs on the UI thread, which a PNG can hold up
    /// for about a second in a debug build.
    fn export_figure_to(&mut self, path: &Path) {
        let Some(figure) = self.figure() else {
            self.errors.push(format!(
                "{}: nothing to export, no curve is shown",
                path.display()
            ));
            return;
        };
        if let Err(e) = figure::write_figure(path, &figure) {
            self.errors.push(e);
        }
    }

    /// Where the Export figure dialog starts: like the CSV export, named
    /// "<file name>_figure.<first supported extension>".
    fn figure_default_location(&self) -> Option<(PathBuf, String)> {
        let ext = figure::EXTENSIONS.first()?;
        self.save_default_location(&format!("_figure.{ext}"))
    }

    fn export_figure_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        for ext in figure::EXTENSIONS {
            dialog = dialog.add_filter(ext.to_ascii_uppercase(), &[ext]);
        }
        if let Some((dir, file_name)) = self.figure_default_location() {
            dialog = dialog.set_directory(dir).set_file_name(file_name);
        }
        if let Some(path) = dialog.save_file() {
            self.export_figure_to(&path);
        }
    }

    /// The curves the plot shows, in drawing order (file order, then curve
    /// order), with the current processing applied.
    fn visible_curves(&self) -> Vec<VisibleCurve<'_>> {
        let baseline = self.processing.baseline();
        let normalize = self.processing.normalize();
        let mut out = Vec::new();
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
                out.push(VisibleCurve {
                    name,
                    points: processing::apply(curve, &baseline, &normalize),
                    color: file.curve_color(ci),
                    path: &file.path,
                    index: ci,
                    dataset: &file.dataset,
                });
            }
        }
        out
    }
}

/// One curve as the plot draws it; see [`UvViewerApp::visible_curves`].
struct VisibleCurve<'a> {
    /// Legend name.
    name: String,
    /// Processed `[x, y]` points.
    points: Vec<[f64; 2]>,
    color: Color32,
    /// Path of the curve's file and the curve's index in it, which together
    /// identify the curve across file reorders.
    path: &'a Path,
    index: usize,
    /// The curve's file, for its units and name.
    dataset: &'a Dataset,
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

/// The x and y axis titles for `dataset`'s curves. Shared by the plot and the
/// CSV headers so they always match.
fn axis_titles(dataset: &Dataset, normalize: Normalize) -> (String, String) {
    let x = axis_label(&dataset.x_unit);
    let y = if normalize == Normalize::None {
        axis_label(&dataset.y_unit)
    } else {
        "Normalized".to_owned()
    };
    (x, y)
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
                    let can_export = self.any_curve_shown();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export CSV…"))
                        .on_disabled_hover_text("No curve is shown")
                        .clicked()
                    {
                        ui.close();
                        self.export_dialog();
                    }
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export figure…"))
                        .on_disabled_hover_text("No curve is shown")
                        .clicked()
                    {
                        ui.close();
                        self.export_figure_dialog();
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

    /// Copies of a fixture in a per-test temporary directory, which is removed
    /// when this guard is dropped (also when the test panics).
    struct TempCopies {
        dir: PathBuf,
        paths: Vec<PathBuf>,
    }

    impl Drop for TempCopies {
        fn drop(&mut self) {
            // Best-effort cleanup: a leftover temp directory must not fail the test.
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl TempCopies {
        /// A fresh, empty per-test temporary directory.
        fn empty(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("uv_viewer-{test}-{}", std::process::id()));
            // Clear a leftover from an earlier run that had the same PID.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempCopies {
                dir,
                paths: Vec::new(),
            }
        }
    }

    /// Copies the single-curve fixture to `count` distinct paths, since
    /// opening the same path twice is ignored.
    fn single_curve_copies(test: &str, count: usize) -> TempCopies {
        // Own the directory before copying, so a failed copy still cleans up.
        let mut copies = TempCopies::empty(test);
        for i in 0..count {
            let path = copies.dir.join(format!("single_{i}.txt"));
            std::fs::copy(testdata(fixtures::single::FILE), &path).expect("copy fixture");
            copies.paths.push(path);
        }
        copies
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
        let copies = single_curve_copies("close", 3);
        let paths = &copies.paths;
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
        let copies = single_curve_copies("reorder", 3);
        let paths = &copies.paths;
        let mut app = UvViewerApp::default();
        app.open_paths(paths[..2].to_vec());
        app.move_file(1, 0);
        app.files.remove(1);
        app.open_paths([paths[2].clone()]);

        let colors = single_colors(&app);
        assert_eq!(colors.len(), 2);
        assert_ne!(colors[0], colors[1]);
    }

    /// Startup arguments are opened on top of the restored state, through the
    /// same duplicate check and error reporting as the Open dialog.
    #[test]
    fn startup_args_open_after_restoring_state() {
        let mut saved = UvViewerApp::default();
        saved.open_paths([testdata(fixtures::multi::FILE)]);
        let json = serde_json::to_string(&saved).expect("serialize");
        let mut app: UvViewerApp = serde_json::from_str(&json).expect("deserialize");
        app.reload_files();

        app.open_startup_args([
            testdata(fixtures::single::FILE).into_os_string(),
            testdata("does_not_exist.txt").into_os_string(),
            // Already restored: must not be added a second time.
            testdata(fixtures::multi::FILE).into_os_string(),
        ]);

        let paths: Vec<_> = app.files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            paths,
            [
                testdata(fixtures::multi::FILE),
                testdata(fixtures::single::FILE)
            ]
        );
        assert_eq!(app.files[1].dataset.curves.len(), 1);
        assert_eq!(app.errors.len(), 1);
    }

    /// Option-like arguments (the app has none; old macOS Finder passes
    /// `-psn_...`) are skipped rather than reported as missing files.
    #[test]
    fn startup_args_skip_option_like_arguments() {
        let mut app = UvViewerApp::default();
        app.open_startup_args(["-psn_0_12345".into(), "--help".into()]);
        assert!(app.files.is_empty());
        assert!(app.errors.is_empty());
    }

    /// A relative argument is stored as an absolute path, so it still loads
    /// when the next session starts in another directory, and it matches the
    /// same file restored under its absolute path.
    #[test]
    fn relative_startup_args_are_made_absolute() {
        // `cargo test` runs from the package root.
        let relative = Path::new("testdata").join(fixtures::single::FILE);
        let mut app = UvViewerApp::default();
        app.open_startup_args([relative.into_os_string()]);
        app.open_paths([testdata(fixtures::single::FILE)]);

        assert_eq!(app.files.len(), 1);
        assert!(app.files[0].path.is_absolute());
        assert!(app.errors.is_empty());
    }

    fn visible_names(app: &UvViewerApp) -> Vec<String> {
        app.visible_curves().into_iter().map(|c| c.name).collect()
    }

    fn opened_single_and_multi() -> UvViewerApp {
        let mut app = UvViewerApp::default();
        app.open_paths([
            testdata(fixtures::single::FILE),
            testdata(fixtures::multi::FILE),
        ]);
        assert!(app.errors.is_empty());
        app
    }

    /// Curves come in file order, then curve order; a multi-curve file names
    /// each curve "file: label", a single-curve file just "file".
    #[test]
    fn visible_curves_are_named_in_plot_order() {
        let app = opened_single_and_multi();
        let expected: Vec<String> = std::iter::once("single_spectrum".to_owned())
            .chain(
                fixtures::multi::LABELS
                    .iter()
                    .map(|label| format!("multi_spectra: {label}")),
            )
            .collect();
        assert_eq!(visible_names(&app), expected);

        let keys: Vec<(&Path, usize)> = app
            .visible_curves()
            .iter()
            .map(|c| (c.path, c.index))
            .collect();
        let single = testdata(fixtures::single::FILE);
        let multi = testdata(fixtures::multi::FILE);
        let expected_keys: Vec<(&Path, usize)> = std::iter::once((single.as_path(), 0))
            .chain((0..fixtures::multi::LABELS.len()).map(|i| (multi.as_path(), i)))
            .collect();
        assert_eq!(keys, expected_keys);
        // The plot used to key lines by `&PathBuf`; `&Path` hashes the same, so
        // legend-hidden state keeps its id.
        assert_eq!(
            egui::Id::new(("curve", multi.as_path(), 1)),
            egui::Id::new(("curve", &multi, 1))
        );
    }

    #[test]
    fn hiding_a_file_hides_all_its_curves() {
        let mut app = opened_single_and_multi();
        app.files[1].visible = false;
        assert_eq!(visible_names(&app), ["single_spectrum"]);

        app.files[1].visible = true;
        app.files[0].visible = false;
        assert_eq!(
            visible_names(&app).len(),
            fixtures::multi::LABELS.len(),
            "only the multi-curve file's curves remain"
        );
        assert!(
            app.visible_curves()
                .iter()
                .all(|c| c.path == testdata(fixtures::multi::FILE))
        );
    }

    #[test]
    fn hiding_one_curve_hides_only_that_curve() {
        let mut app = opened_single_and_multi();
        app.files[1].curves[2].visible = false;
        let expected: Vec<String> = std::iter::once("single_spectrum".to_owned())
            .chain(
                fixtures::multi::LABELS
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| i != 2)
                    .map(|(_, label)| format!("multi_spectra: {label}")),
            )
            .collect();
        assert_eq!(visible_names(&app), expected);
        assert!(
            app.visible_curves()
                .iter()
                .all(|c| !(c.path == testdata(fixtures::multi::FILE) && c.index == 2))
        );
    }

    /// The points are the processed values and the colours the file's curve
    /// colours, including a per-curve override.
    #[test]
    fn visible_curves_carry_processed_points_and_colors() {
        let mut app = opened_single_and_multi();
        app.processing.baseline_kind = BaselineKind::RangeMean;
        app.processing.normalize_kind = NormalizeKind::Max;
        app.files[1].curves[4].color = Some(Color32::RED);
        let baseline = app.processing.baseline();
        let normalize = app.processing.normalize();

        let curves = app.visible_curves();
        assert_eq!(curves.len(), 1 + fixtures::multi::LABELS.len());
        for curve in &curves {
            let file = app
                .files
                .iter()
                .find(|f| f.path == curve.path)
                .expect("curve belongs to an open file");
            let expected =
                processing::apply(&file.dataset.curves[curve.index], &baseline, &normalize);
            assert_eq!(curve.points, expected, "{}", curve.name);
            assert_eq!(curve.color, file.curve_color(curve.index), "{}", curve.name);
        }
        assert_eq!(curves[1 + 4].color, Color32::RED);
    }

    /// Reads back an exported file: checks the byte order mark and the CRLF
    /// line ends, and returns the lines split into cells.
    fn read_export(path: &Path) -> Vec<Vec<String>> {
        let bytes = std::fs::read(path).expect("read back export");
        let text = bytes
            .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
            .expect("starts with UTF-8 BOM");
        let text = std::str::from_utf8(text).expect("UTF-8");
        let body = text.strip_suffix("\r\n").expect("ends with CRLF");
        body.split("\r\n")
            .map(|line| line.split(',').map(str::to_owned).collect())
            .collect()
    }

    /// File -> Export CSV writes exactly the shown curves (file and curve both
    /// visible), with the processing applied and the plot's axis titles.
    #[test]
    fn export_writes_the_shown_curves_processed() {
        let tmp = TempCopies::empty("export_shown");
        let path = tmp.dir.join("out.csv");
        let mut app = opened_single_and_multi();
        app.files[1].curves[2].visible = false;
        app.processing.baseline_kind = BaselineKind::RangeMean;
        app.processing.normalize_kind = NormalizeKind::Max;

        app.export_to(&path);
        assert!(app.errors.is_empty(), "{:?}", app.errors);

        let curves = app.visible_curves();
        assert_eq!(curves.len(), fixtures::multi::LABELS.len());
        let rows = read_export(&path);
        let longest = curves.iter().map(|c| c.points.len()).max().unwrap();
        assert_eq!(rows.len(), 2 + longest);

        let names: Vec<String> = curves
            .iter()
            .flat_map(|c| [c.name.clone(), c.name.clone()])
            .collect();
        assert_eq!(rows[0], names);
        let hidden = format!("multi_spectra: {}", fixtures::multi::LABELS[2]);
        assert!(!rows[0].contains(&hidden));

        let (x_title, y_title) = app.axis_titles().expect("files are open");
        assert_eq!(x_title, "Wavelength / nm");
        assert_eq!(y_title, "Normalized");
        let headers: Vec<String> = curves
            .iter()
            .flat_map(|_| [x_title.clone(), y_title.clone()])
            .collect();
        assert_eq!(rows[1], headers);

        for (i, row) in rows[2..].iter().enumerate() {
            assert_eq!(row.len(), 2 * curves.len(), "row {i}");
            for (k, c) in curves.iter().enumerate() {
                match c.points.get(i) {
                    Some([x, y]) => {
                        let got_x: f64 = row[2 * k].parse().expect("x parses");
                        let got_y: f64 = row[2 * k + 1].parse().expect("y parses");
                        assert_eq!(got_x.to_bits(), x.to_bits(), "row {i} curve {k} x");
                        assert_eq!(got_y.to_bits(), y.to_bits(), "row {i} curve {k} y");
                    }
                    None => assert_eq!(row[2 * k..2 * k + 2], ["", ""], "row {i} curve {k}"),
                }
            }
        }
    }

    /// Without normalization the y header is the y axis title, as in the plot.
    #[test]
    fn export_without_normalization_uses_the_y_axis_title() {
        let tmp = TempCopies::empty("export_raw");
        let path = tmp.dir.join("out.csv");
        let mut app = opened_single_and_multi();
        app.export_to(&path);
        assert!(app.errors.is_empty(), "{:?}", app.errors);

        let rows = read_export(&path);
        let (x_title, y_title) = app.axis_titles().expect("files are open");
        assert_eq!(y_title, "Absorbance");
        assert_eq!(rows[1][..2], [x_title, y_title]);
        assert!(rows[1].iter().skip(1).step_by(2).all(|h| h == "Absorbance"));
    }

    /// A failed write is shown in the errors list instead of panicking.
    #[test]
    fn export_failures_are_reported() {
        let tmp = single_curve_copies("export_fail", 1);
        let mut app = opened_single_and_multi();

        app.export_to(&tmp.dir.join("missing").join("out.csv"));
        assert_eq!(app.errors.len(), 1, "{:?}", app.errors);

        // A path below a regular file.
        app.export_to(&tmp.paths[0].join("out.csv"));
        assert_eq!(app.errors.len(), 2, "{:?}", app.errors);
    }

    /// With nothing shown there is nothing to export: no file is written and
    /// the reason is shown, rather than writing a CSV with two empty lines.
    #[test]
    fn export_with_no_shown_curve_writes_nothing() {
        let tmp = TempCopies::empty("export_none");
        let path = tmp.dir.join("out.csv");
        let mut app = opened_single_and_multi();
        app.files[0].visible = false;
        app.files[1]
            .curves
            .iter_mut()
            .for_each(|c| c.visible = false);

        app.export_to(&path);
        assert_eq!(app.errors.len(), 1, "{:?}", app.errors);
        assert!(!path.exists());
        assert_eq!(app.export_default_location(), None);
    }

    /// The save dialog starts in the folder of the first shown file, with
    /// "<file name>_export.csv" as the name.
    #[test]
    fn export_dialog_defaults_follow_the_first_shown_file() {
        let mut app = opened_single_and_multi();
        let dir = testdata(fixtures::single::FILE)
            .parent()
            .expect("fixture has a folder")
            .to_path_buf();
        assert_eq!(
            app.export_default_location(),
            Some((dir.clone(), "single_spectrum_export.csv".to_owned()))
        );

        app.files[0].visible = false;
        assert_eq!(
            app.export_default_location(),
            Some((dir, "multi_spectra_export.csv".to_owned()))
        );
    }

    /// File -> Export figure draws exactly the shown curves, with their plot
    /// colours and legend names and the plot's axis titles.
    #[test]
    fn export_figure_writes_the_shown_curves() {
        let tmp = TempCopies::empty("figure_shown");
        let path = tmp.dir.join("out.svg");
        let mut app = opened_single_and_multi();
        app.files[1].curves[2].visible = false;
        app.files[1].curves[4].color = Some(Color32::RED);
        app.processing.normalize_kind = NormalizeKind::Max;

        let figure = app.figure().expect("curves are shown");
        let (x_title, y_title) = app.axis_titles().expect("files are open");
        assert_eq!(
            (figure.x_title.as_str(), figure.y_title.as_str()),
            ("Wavelength / nm", "Normalized")
        );
        assert_eq!((&figure.x_title, &figure.y_title), (&x_title, &y_title));
        assert!(figure.legend);
        let expected: Vec<(String, Vec<[f64; 2]>, Color32)> = app
            .visible_curves()
            .into_iter()
            .map(|c| (c.name, c.points, c.color))
            .collect();
        let got: Vec<(String, Vec<[f64; 2]>, Color32)> = figure
            .curves
            .iter()
            .map(|c| (c.name.clone(), c.points.clone(), c.color))
            .collect();
        assert_eq!(got, expected);

        app.export_figure_to(&path);
        assert!(app.errors.is_empty(), "{:?}", app.errors);
        let svg = std::fs::read_to_string(&path).expect("read back figure");
        assert!(svg.contains("<svg"), "not an SVG file");
        for (name, _, _) in &expected {
            assert!(svg.contains(name.as_str()), "legend lacks {name:?}");
        }
        let hidden = format!("multi_spectra: {}", fixtures::multi::LABELS[2]);
        assert!(!svg.contains(&hidden), "hidden curve {hidden:?} drawn");
        assert!(svg.contains(&x_title) && svg.contains(&y_title));
    }

    /// The figure's legend follows "Show legend".
    #[test]
    fn export_figure_legend_follows_show_legend() {
        let mut app = opened_single_and_multi();
        app.show_legend = false;
        assert!(!app.figure().expect("curves are shown").legend);
    }

    /// Failed figure writes are shown in the errors list instead of panicking.
    #[test]
    fn export_figure_failures_are_reported() {
        let tmp = single_curve_copies("figure_fail", 1);
        let mut app = opened_single_and_multi();

        let txt = tmp.dir.join("out.txt");
        app.export_figure_to(&txt);
        assert_eq!(app.errors.len(), 1, "{:?}", app.errors);
        assert!(!txt.exists());

        // A path below a regular file.
        let ext = figure::EXTENSIONS[0];
        app.export_figure_to(&tmp.paths[0].join(format!("out.{ext}")));
        assert_eq!(app.errors.len(), 2, "{:?}", app.errors);
    }

    /// With nothing shown the item is disabled; a call anyway writes nothing
    /// and says why.
    #[test]
    fn export_figure_with_no_shown_curve_writes_nothing() {
        let tmp = TempCopies::empty("figure_none");
        let path = tmp.dir.join("out.svg");
        let mut app = opened_single_and_multi();
        assert!(app.any_curve_shown());
        app.files[0].visible = false;
        app.files[1]
            .curves
            .iter_mut()
            .for_each(|c| c.visible = false);
        assert!(!app.any_curve_shown());
        assert!(app.visible_curves().is_empty());

        app.export_figure_to(&path);
        assert_eq!(app.errors.len(), 1, "{:?}", app.errors);
        assert!(!path.exists());
        assert_eq!(app.figure_default_location(), None);
        assert!(!UvViewerApp::default().any_curve_shown());
    }

    /// The figure dialog starts where the CSV one does, with
    /// "<file name>_figure.<first supported extension>" as the name.
    #[test]
    fn figure_dialog_defaults_follow_the_first_shown_file() {
        let mut app = opened_single_and_multi();
        app.files[0].visible = false;
        let dir = testdata(fixtures::multi::FILE)
            .parent()
            .expect("fixture has a folder")
            .to_path_buf();
        assert_eq!(
            app.figure_default_location(),
            Some((
                dir,
                format!("multi_spectra_figure.{}", figure::EXTENSIONS[0])
            ))
        );
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
