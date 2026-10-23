# UV Viewer

[English README](README.md)

JASCO の分光光度計（V-760 など）からテキスト出力した UV-Vis スペクトルを表示するデスクトップアプリです。[egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) で作られています。複数のファイルを開いて 1 つのグラフに重ねて表示し、すべての曲線にベースライン補正と規格化をかけ、複数スキャンのファイルをグラデーションで色分けできます。

egui のテンプレートから作り始めたプロジェクトで、クレート名・バイナリ名・リリース成果物名（`egui_template`、`egui-template-…`）はまだ変更していません。

`v*` タグを push すると、GitHub Actions が **Windows / macOS / Linux 向けインストーラ**をビルドして Release に公開します。

| プラットフォーム | 成果物 | 形式 |
| --- | --- | --- |
| Windows | `egui-template-<version>-windows-x64-setup.exe` | Inno Setup インストーラ |
| Windows | `egui-template-<version>-windows-x64.exe` | ポータブル実行ファイル |
| macOS | `egui-template-<version>-macos-universal.dmg` | Applications へドラッグする形式のディスクイメージ（Intel + Apple Silicon ユニバーサル） |
| macOS | `egui-template-<version>-macos-universal` | ポータブル実行ファイル |
| Linux | `egui-template-<version>-linux-x86_64.AppImage` | 単体で動作する AppImage |
| Linux | `egui-template_<version>_amd64.deb` | Debian / Ubuntu パッケージ |
| Linux | `egui-template-<version>-linux-x86_64` | ポータブル実行ファイル |

## 対応ファイル

JASCO の分光光度計のテキスト出力を読み込みます。

- **文字コード** — CP932（Shift-JIS）。
- **ヘッダ** — `XYDATA` と書かれた行までの `KEY<TAB>VALUE` 形式の行。`XUNITS` と `YUNITS` は軸タイトルになります（`NANOMETERS` → *Wavelength / nm*、`ABSORBANCE` → *Absorbance*、`Temperature…` → *Temperature / °C*。それ以外の単位はそのまま表示）。`XYDATA` 行が無いファイルや、その後に数値データが 1 つも無いファイルはエラーになります。
- **1 列形式** — 1 行に `x<TAB>y` が 1 組。曲線は 1 本で、ファイル名が付きます。
- **複数列形式** — タブで始まりスキャンごとのラベル（通常は温度。例: `<TAB>85<TAB>70…`）を並べた行のあとに、`x<TAB>y1<TAB>y2…` の行が続きます。列ごとに 1 本の曲線になり、そのラベルが名前になります。
- データは最初の空行または `#####` で始まる行で終わるため、末尾の `##### Extended Information` セクションは無視されます。

x の値は波長でも、装置が出力する別の量でも構いません（例: 単一波長での温度スキャンの温度）。

## 使いかた

### ファイルを開く

- **File → Open…**、またはサイドパネルの *Files* の横にある **Open…** ボタンでファイルダイアログが開きます（フィルタは *JASCO text*（`*.txt`）と *All files*。複数ファイルを一度に選択できます）。
- ウィンドウにファイルをドロップしても開けます。
- すでに開いているファイルは二重には開きません。読み込みに失敗したファイルは、サイドパネル下部にエラーメッセージとして表示され、**Dismiss** を押すと消えます。
- **File → Close all** ですべてのファイルを閉じます。ファイルの行の **✖** ボタンでそのファイルだけを閉じます。

### ベースラインと規格化

*Processing* の設定はすべての曲線に、ベースライン → 規格化の順で適用されます。

- **Baseline**
  - **None**
  - **Range mean** — x が 2 つの値の間にある点の平均値を引きます。範囲内に点が 1 つも無い場合は、範囲の中央で補間した値を引きます。
  - **Two-point** — 2 つの x 位置で曲線を通る直線を引きます。
- **Normalize**
  - **None**
  - **Max = 1** — （ベースライン補正後の）曲線の最大値で割ります。
  - **Value at x = 1** — 指定した x での曲線の値で割ります。

*Two-point* と *Value at x* の位置での曲線の値は線形補間で求めるため、位置がデータ点と一致している必要はありません。*Two-point* や *Value at x* の位置が曲線の x の範囲外にある場合、その曲線ではその処理を行いません（曲線は表示されたままです）。*Range mean* は、範囲が曲線の外にはみ出していても範囲内にある点の平均を引きます。処理を行わないのは、範囲内に点が 1 つも無く、範囲の中央も曲線の外にある場合だけです。割る値が（ほぼ）0 の場合も規格化は行いません。また、最大値が正でない曲線には *Max = 1* を適用しません（その値で割ると曲線が上下反転するためです）。規格化している間、y 軸のタイトルは *Normalized* になります。

### ファイル・曲線・色

- ファイルごとに、すべての曲線の表示・非表示を切り替えるチェックボックスと、ドラッグで一覧内の位置を並べ替える **☰** ハンドルがあります。
- ファイルを展開すると、色の設定と、曲線ごとの行（表示チェックボックスと色ボタン）が表示されます。**Show all** / **Hide all** でそのファイルの全曲線をまとめて切り替えます。
- ファイルごとの色モード:
  - **Gradient** — カラーマップ（*Viridis*、*Jet*、*Coolwarm*、*Blue → Red*）に沿って曲線を色分けします。曲線のラベルがすべて数値の場合（複数列ファイルのスキャン温度など）はその値に従ってカラーマップ上の位置を決めるため、昇温・降温どちらの測定でも最も高温のスキャンがカラーマップの上端になります。そうでない場合はファイル内の順に均等に割り当てます。
  - **Single** — そのファイルの全曲線を 1 色にします。
- 複数列ファイルは *Gradient*（*Coolwarm*）で、1 本だけのファイルは 8 色のパレットのうち、開いているどのファイルも使っていない最初の色で始まります（8 色すべてが使われているときは、使われている数が最も少ない色）。
- 曲線ごとに色を選ぶと、ファイルの色モードより優先されます。**↺**（ファイル全体なら **Reset colors**）でファイルの色に戻ります。
- **Show legend** でグラフの凡例を切り替えます。軸タイトルは最初に表示されているファイルから取ります。

### CSV の書き出し

**File → Export CSV…** で、表示中の曲線を CSV ファイルに書き出します。値には *Processing* の設定が適用されています。つまり、ベースライン補正と規格化をしたあとの、グラフに描かれているとおりの値です。

- 書き出されるのは、ファイルのチェックボックスと曲線のチェックボックスの両方がオンの曲線です。グラフの凡例をクリックして隠した曲線も書き出されます。
- 表示中の曲線が 1 本も無いときは、このメニュー項目は押せません。
- 保存ダイアログ（フィルタは *CSV*（`*.csv`））は、最初に表示されている曲線のファイルがあるフォルダで開き、ファイル名は `<そのファイルの名前>_export.csv` になっています（`spectrum.txt` → `spectrum_export.csv`）。
- 書き込めなかった場合は、読み込みに失敗したファイルと同じように、サイドパネル下部にエラーとして表示されます。

曲線ごとに x と y の 2 列を、描画順（一覧でのファイルの順、ファイル内では曲線の順）に並べます。グラフの凡例は名前のアルファベット順に並ぶため、凡例の順と列の順が異なることがあります（例: 複数列ファイルの温度ごとのスキャン）。

1. 1 行目 — 曲線の凡例名を、その曲線の 2 列の両方に書きます。拡張子を除いたファイル名、複数列ファイルの曲線なら `<ファイル名>: <ラベル>` です。
2. 2 行目 — その曲線のファイルの x と y のタイトルです。グラフの軸と同じように `XUNITS` / `YUNITS` から決まります（規格化している間、y は *Normalized*）。グラフの軸タイトルは最初に表示されているファイルだけで決まるため、単位の違うファイルを一緒に表示しているときは、この見出しがグラフの軸タイトルと異なることがあります。
3. 3 行目以降 — 1 行に 1 点です。曲線ごとに自分の x を持つので、x の刻みが違う曲線もそれぞれの x と組になったままです。最も長い曲線より短い曲線では、終わったあとの行の 2 つのセルが空欄になります。

書式:

- 区切りはカンマで、最後の行も含めてすべての行が CRLF で終わります。
- 数値の小数点は、システムのロケールにかかわらず常に `.` です。指数表記にはならず、精度も落としません（読み戻すとビューアが持っていた値と完全に一致します）。NaN と無限大は空欄になります。
- カンマ・ダブルクォート・改行を含む名前や見出しはダブルクォートで囲み、`"` は 2 つ重ねます。
- 文字コードは UTF-8（BOM 付き）です。このため、日本語版 Windows の Excel でも日本語のファイル名が文字化けしません。

pandas で読むときは、2 行の見出しを両方指定し、エンコーディングに `utf-8-sig`（BOM を取り除きます）を指定してください。

```python
import pandas as pd

df = pd.read_csv("spectrum_export.csv", header=[0, 1], encoding="utf-8-sig")
spectrum = df["spectrum"].dropna()  # 1 本の曲線の x 列と y 列（空欄（短い曲線の後ろの空欄、NaN・無限大）を含む行を除く）
```

列名は `("spectrum", "Wavelength / nm")` のような `(名前, 見出し)` の組になります。

### 保存される状態

開いているファイルと設定は eframe の `persistence` 機能で保存され、次回起動時に復元されます。対象は、ファイルのパスと並び順、ファイルと曲線の表示状態、色モードと曲線の色、処理の設定、凡例の表示です。保存されるのはパスだけでデータは保存しないため、起動時にファイルをディスクから読み直します。読めなくなったファイルは一覧から外され、エラーとして表示されます。曲線ごとの設定は、曲線の本数が変わっていない限り引き継がれ、変わった場合はリセットされます。

## 含まれているもの

- **状態の永続化** — eframe の `persistence` 機能により、開いているファイルと設定が再起動後も復元されます（[保存される状態](#保存される状態) を参照）。
- **全プラットフォーム分のアイコン** — `resources/` に `icon.png`（ウィンドウ + Linux）・`icon.ico`（Windows の `.exe`）・`icon.icns`（macOS バンドル）を用意しています。
- **宣言的な macOS バンドル** — `cargo bundle` が `[package.metadata.bundle]` から `Info.plist` を生成するため、plist を手で管理する必要がありません。
- **Wayland でのドラッグ&ドロップ** — `[patch.crates-io]` で `winit` / `egui-winit` のフォークを指定しています（本家は Wayland のネイティブ DnD に未対応）。
- **テスト可能なアプリロジック** — アプリ本体を `main.rs` ではなく `src/lib.rs` に置いているため、`cargo test` から参照できます。
- **CI** — 3 プラットフォームで `cargo fmt` / `cargo clippy -D warnings` / `cargo test` を実行します。

> **Rust 1.92 以上**が必要です。edition 2024 自体は 1.85 で足りますが、Wayland DnD 用の
> `egui-winit` フォークが要求バージョンを引き上げています。

## はじめかた

```bash
cargo run
```

コードは役割ごとに分かれています。

- [`src/jasco.rs`](src/jasco.rs) — JASCO テキスト出力のパーサ。
- [`src/processing.rs`](src/processing.rs) — ベースライン補正と規格化。
- [`src/color.rs`](src/color.rs) — カラーマップ、単色用パレット、グラデーション上の位置。
- [`src/export.rs`](src/export.rs) — 表示中の曲線の CSV 書き出し。
- [`src/app.rs`](src/app.rs) — アプリの状態、UI、永続化。

## 自分のアプリにする

名前は複数のファイルから相互に参照しているため、次の順で変更してください。

1. **`Cargo.toml`** — `name` / `version` / `description` / `authors` / `license` / `repository` と `[package.metadata.deb]` セクション。
2. **`src/lib.rs`** — `APP_NAME`（ウィンドウタイトルと永続化キー）。
3. **`.github/workflows/release.yml`** — `BIN_NAME`（`Cargo.toml` の `name` と一致させる）と `APP_NAME`（成果物のファイル名に使用）。
4. **`installer/windows.iss`** — `MyAppName` / `MyAppPublisher` / `MyAppURL` / `MyAppExeName`、そして **`AppId` の GUID を新しく生成**してください。`AppId` は Windows がアップグレードを判別するための識別子です。一度リリースしたあとは絶対に変更しないでください。
5. **`Cargo.toml` の `[package.metadata.bundle]`** — `name` / `identifier`（自分が管理するドメインを使用）/ `category` / 各種説明文。あわせて `release.yml` の `macos` ジョブ内にハードコードされている `egui Template.app` のパスも、`bundle.name` と一致するように更新してください。
6. **`installer/linux/egui-template.desktop`** — ファイル名自体もリネームし、`Exec` と `Icon` を更新。
7. **`installer/linux/AppRun`** — `exec` 行のバイナリ名。
8. **`resources/icon.png`** / **`resources/icon.ico`** / **`resources/icon.icns`** — 自分のアイコンに差し替え。PNG は 1024×1024、`.ico` はマルチ解像度（16〜256 px）、`.icns` は最低でも 32/128/256/512/1024 px を含む状態を保ってください。
9. **`LICENSE`** — 著作権者名を記入。

## リリースする

```bash
git tag v0.1.0
git push origin v0.1.0
```

ワークフローはタグからバージョンを取り出し（`v0.1.0` → `0.1.0`）、インストーラ・`.deb`・`.dmg` に渡します。つまりリリースのバージョンは `Cargo.toml` の `version` ではなくタグが基準です。とはいえ About ダイアログの表示が合わなくなるので、`Cargo.toml` 側も上げておくことをおすすめします。

手動実行（**Actions → Release → Run workflow**）した場合は、すべて `0.0.0-dev` としてビルドし、Release を作らずに成果物だけをアップロードします。ワークフロー自体の動作確認に便利です。

## 署名と公証（notarization）について

macOS の `.app` は **アドホック署名**（`codesign --sign -`）です。これは Apple Silicon で「アプリが破損しています」と言われずに起動させるために必要な最低限の署名で、公証は通っていません。そのため利用者は初回のみ右クリック →「開く」で起動する必要があります。正式に配布するには有償の Apple Developer アカウント、Developer ID による `codesign`、`xcrun notarytool submit` が必要です。

Windows のインストーラも未署名なので、ダウンロード時に SmartScreen の警告が出ます。署名にはコードサイニング証明書と `signtool` が必要です。

## ローカルでインストーラをビルドする

```bash
cargo build --release

# Windows（Inno Setup 6 が必要）
iscc installer\windows.iss              # -> dist\

# Linux .deb（cargo-deb が必要）
cargo install cargo-deb && cargo deb

# macOS .app（cargo-bundle が必要）
cargo install cargo-bundle && cargo bundle --release
# さらに .dmg を作る場合は `brew install create-dmg` が必要です。
# 詳細は .github/workflows/release.yml の macos ジョブを参照してください
```

## act でワークフローを試す

[`act`](https://github.com/nektos/act) を使うと Docker 上でワークフローをローカル実行できます。

```bash
act -W .github/workflows/ci.yml -j lint
act -W .github/workflows/release.yml -j linux
```

`act` で動かせるのは Linux ジョブだけです。`windows` と `macos` ジョブは実際のランナーが必要です。使用するランナーイメージは [`.actrc`](.actrc) に固定しています。

## Linux でのビルドに必要なパッケージ

```bash
sudo apt-get install -y \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libegl1-mesa-dev \
  libgtk-3-dev libssl-dev
```

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
