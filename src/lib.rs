//! UV-Vis spectrum viewer for JASCO text exports.
//!
//! Keeping the app in a library (rather than entirely in `main.rs`) means the
//! parsing, processing and state logic stay unit-testable.

pub mod app;
pub mod color;
pub mod export;
pub mod figure;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod jasco;
pub mod processing;
pub mod trace;

pub use app::UvViewerApp;

/// Display name, used for the window title and the eframe persistence key.
pub const APP_NAME: &str = "UV Viewer";
