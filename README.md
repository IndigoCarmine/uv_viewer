# UV Viewer

[日本語版 README](README_ja.md)

A desktop viewer for UV-Vis spectra exported as text from JASCO spectrometers (e.g. the V-760), built with [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe). Open several files, overlay their curves on one plot, apply a baseline correction and normalisation to all of them, and colour multi-scan files with a gradient.

The project started from an egui template, and the crate, binary and release artifact names (`egui_template`, `egui-template-…`) have not been renamed yet.

Push a `v*` tag and GitHub Actions builds and publishes **installers for Windows, macOS and Linux**.

| Platform | Artifact | Format |
| --- | --- | --- |
| Windows | `egui-template-<version>-windows-x64-setup.exe` | Inno Setup installer |
| Windows | `egui-template-<version>-windows-x64.exe` | Portable binary |
| macOS | `egui-template-<version>-macos-universal.dmg` | Drag-to-Applications disk image (universal: Intel + Apple Silicon) |
| macOS | `egui-template-<version>-macos-universal` | Portable binary |
| Linux | `egui-template-<version>-linux-x86_64.AppImage` | Self-contained AppImage |
| Linux | `egui-template_<version>_amd64.deb` | Debian/Ubuntu package |
| Linux | `egui-template-<version>-linux-x86_64` | Portable binary |

## Supported files

The viewer reads the text export of JASCO spectrometers:

- **Encoding** — CP932 (Shift-JIS).
- **Header** — `KEY<TAB>VALUE` lines up to a line reading `XYDATA`. `XUNITS` and `YUNITS` become the axis titles (`NANOMETERS` → *Wavelength / nm*, `ABSORBANCE` → *Absorbance*, `Temperature…` → *Temperature / °C*; any other unit is shown as written). A file without an `XYDATA` line, or without any numeric data after it, is rejected with an error.
- **Single column** — one `x<TAB>y` pair per line: one curve, named after the file.
- **Multi column** — a row that starts with a tab and lists one label per scan (usually the temperature, e.g. `<TAB>85<TAB>70…`), followed by `x<TAB>y1<TAB>y2…` lines: one curve per column, named by its label.
- The data ends at the first empty line or `#####` line, so a trailing `##### Extended Information` section is ignored.

The x values can be wavelengths or anything else the instrument exports, such as temperatures in a single-wavelength temperature scan.

## Using the viewer

### Opening files

- **File → Open…**, or the **Open…** button next to *Files* in the side panel, opens a file dialog (filters *JASCO text*, `*.txt`, and *All files*; several files can be selected at once).
- Files can also be dropped onto the window.
- A file that is already open is not opened a second time. Files that fail to load are reported as error messages at the bottom of the side panel until you click **Dismiss**.
- **File → Close all** closes every file; the **✖** button in a file's row closes just that file.

### Baseline and normalisation

The *Processing* settings apply to every curve, baseline first, then normalisation:

- **Baseline**
  - **None**
  - **Range mean** — subtracts the mean of the points whose x lies between the two values. If no point falls in the range, the value interpolated at the middle of the range is subtracted instead.
  - **Two-point** — subtracts the straight line through the curve at the two x values.
- **Normalize**
  - **None**
  - **Max = 1** — divides by the maximum of the (baseline-corrected) curve.
  - **Value at x = 1** — divides by the curve's value at the given x.

The curve's value at a *Two-point* or *Value at x* position is interpolated linearly, so the position does not have to match a data point. A *Two-point* or *Value at x* position outside a curve's x range skips that step for that curve (the curve is still shown). *Range mean* averages whichever points fall in the range, even if the range extends past the curve, and is skipped only when no point falls in the range and its middle is also outside the curve. Normalisation is also skipped when the divisor is (almost) zero. While normalisation is on, the y axis is titled *Normalized*.

### Files, curves and colours

- Each file has a checkbox to show or hide all its curves, and a **☰** handle to drag it to a new position in the list.
- Expanding a file shows its colour settings and one row per curve, with a visibility checkbox and a colour button. **Show all** / **Hide all** toggle every curve of the file.
- Colour mode per file:
  - **Gradient** — curves are coloured along a colormap: *Viridis*, *Jet*, *Coolwarm* or *Blue → Red*. When every curve label is a number (such as the scan temperatures of a multi-column file), a curve's position on the colormap follows that number, so the hottest scan always gets the top of the colormap whether the run was heated or cooled. Otherwise the curves are spread evenly in file order.
  - **Single** — every curve of the file gets one colour.
- Multi-column files start in *Gradient* mode with *Coolwarm*; single-curve files get the first colour of an 8-colour palette that no open file is using (once all 8 are taken, the least-used one).
- Picking a colour for an individual curve overrides the file's colour mode; **↺** (or **Reset colors** for the whole file) goes back to the file's colours.
- **Show legend** toggles the plot legend. The axis titles come from the first visible file.

### Saved state

The open files and the settings are saved by eframe's `persistence` feature and restored on the next start: the file paths and their order, file and curve visibility, colour modes and curve colours, the processing settings and the legend toggle. Only the paths are stored, not the data, so the files are re-read from disk on startup. A file that can no longer be read is removed from the list and reported as an error. A file's per-curve settings are kept as long as it still has the same number of curves; otherwise they are reset.

## What's in the box

- **Persistent state** — the open files and settings survive restarts via eframe's `persistence` feature (see [Saved state](#saved-state)).
- **Icons for every platform** — `resources/` holds `icon.png` (window + Linux), `icon.ico` (Windows `.exe`) and `icon.icns` (macOS bundle).
- **Declarative macOS bundling** — `cargo bundle` generates `Info.plist` from `[package.metadata.bundle]`, so there is no plist to hand-maintain.
- **Wayland drag-and-drop** — `[patch.crates-io]` pins `winit`/`egui-winit` forks that implement native DnD on Wayland (upstream does not).
- **Unit-testable app logic** — the app lives in `src/lib.rs`, not `main.rs`, so `cargo test` can reach it.
- **CI** — `cargo fmt`, `cargo clippy -D warnings` and `cargo test` on all three platforms.

> **Rust 1.92+** is required. Edition 2024 alone needs only 1.85, but the `egui-winit`
> fork used for Wayland DnD raises the floor.

## Getting started

```bash
cargo run
```

The code is split by concern:

- [`src/jasco.rs`](src/jasco.rs) — parser for the JASCO text export.
- [`src/processing.rs`](src/processing.rs) — baseline correction and normalisation.
- [`src/color.rs`](src/color.rs) — colormaps, the single-colour palette and gradient positions.
- [`src/app.rs`](src/app.rs) — application state, user interface and persistence.

## Making it yours

Rename in this order; the names are cross-referenced between files:

1. **`Cargo.toml`** — `name`, `version`, `description`, `authors`, `license`, `repository`, and the `[package.metadata.deb]` block.
2. **`src/lib.rs`** — `APP_NAME` (window title and persistence key).
3. **`.github/workflows/release.yml`** — `BIN_NAME` (must match `Cargo.toml`'s `name`) and `APP_NAME` (used in artifact filenames).
4. **`installer/windows.iss`** — `MyAppName`, `MyAppPublisher`, `MyAppURL`, `MyAppExeName`, and **generate a fresh `AppId` GUID**. The `AppId` is how Windows recognises an upgrade; once you have shipped a release, never change it.
5. **`Cargo.toml` `[package.metadata.bundle]`** — `name`, `identifier` (use a domain you control), `category`, descriptions. Also update the hard-coded `egui Template.app` paths in the `macos` job of `release.yml`, which must match `bundle.name`.
6. **`installer/linux/egui-template.desktop`** — rename the file too, and update `Exec`/`Icon`.
7. **`installer/linux/AppRun`** — the binary name in the `exec` line.
8. **`resources/icon.png`**, **`resources/icon.ico`** and **`resources/icon.icns`** — replace with your own art. Keep the PNG at 1024×1024, the `.ico` multi-resolution (16–256 px), and the `.icns` containing at least the 32/128/256/512/1024 px variants.
9. **`LICENSE`** — fill in the copyright holder.

## Cutting a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow derives the version from the tag (`v0.1.0` → `0.1.0`) and feeds it to the installer, the `.deb` and the `.dmg`, so `Cargo.toml`'s `version` is not the source of truth for releases. Bump it anyway to keep the About dialog honest.

Running the workflow manually (**Actions → Release → Run workflow**) builds everything as `0.0.0-dev` and uploads the artifacts *without* publishing a release — useful for testing changes to the workflow.

## Signing and notarisation

The macOS bundle is **ad-hoc signed** (`codesign --sign -`). That is enough for the app to launch at all on Apple Silicon, but it is not notarised, so on first launch users must right-click the app and choose **Open**. Proper distribution needs a paid Apple Developer account, `codesign` with a Developer ID, and `xcrun notarytool submit`.

The Windows installer is unsigned, so SmartScreen will warn on download. Signing needs a code-signing certificate and `signtool`.

## Building installers locally

```bash
cargo build --release

# Windows (needs Inno Setup 6)
iscc installer\windows.iss              # -> dist\

# Linux .deb (needs cargo-deb)
cargo install cargo-deb && cargo deb

# macOS .app (needs cargo-bundle)
cargo install cargo-bundle && cargo bundle --release
# ...and a .dmg on top of it (needs `brew install create-dmg`);
# see the `macos` job in .github/workflows/release.yml
```

## Testing workflows with act

[`act`](https://github.com/nektos/act) runs the workflows in Docker locally:

```bash
act -W .github/workflows/ci.yml -j lint
act -W .github/workflows/release.yml -j linux
```

`act` only runs the Linux jobs — `windows` and `macos` need real runners. The runner image is pinned in [`.actrc`](.actrc).

## Linux build dependencies

Building on Linux needs the usual windowing/GL headers:

```bash
sudo apt-get install -y \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libegl1-mesa-dev \
  libgtk-3-dev libssl-dev
```

## License

MIT — see [LICENSE](LICENSE).
