//! Sync through iCloud Drive (or another cloud folder) and earlier versions of saved workbooks:
//! the GridCraft side of `craft-sync`.
//!
//! - File › Options › Save shows the settings ([`options`]), this workbook's earlier versions and
//!   the copies the cloud kept of it, each with Open.
//! - [`tick`] (every frame, in `logic`) mirrors the AI folder every Craft app shares
//!   (`ArtCraft/AI`: each file's conversation, Save chat with file) with the cloud folder every few
//!   minutes, in the background, and says when the cloud kept another copy of the open workbook.
//! - [`before_save`] (the engine's save hook) keeps the workbook as it was before a save
//!   replaces it.
//!
//! The settings are shared by every Craft app (`ArtCraft/sync.json`). The web build has no files
//! of its own and syncs nothing.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use craft_sync::Settings;
use egui::{RichText, Ui};

use crate::SheetApp;
use crate::theme::{self, Tokens};

/// The most versions listed on the settings page.
const MAX_LISTED: usize = 15;

struct State {
    settings: Settings,
    /// The workbook last looked at for cloud copies.
    checked: Option<String>,
    /// The folder being typed on the settings page.
    folder_text: Option<String>,
    /// A version or copy chosen on the settings page, opened by the next [`tick`] (outside the
    /// File view, which then closes).
    open: Option<PathBuf>,
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    let state = STATE.get_or_init(|| Mutex::new(State { settings: Settings::load(), checked: None, folder_text: None, open: None }));
    f(&mut state.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The sync settings, as last saved.
pub fn settings() -> Settings {
    with(|s| s.settings.clone())
}

/// Before a save replaces the file at `path`: keep the file as it was among its earlier versions.
pub fn before_save(path: &str) {
    let s = settings();
    if let Some(root) = s.versions_dir()
        && let Err(e) = craft_sync::versions::snapshot(&root, Path::new(path), s.keep_versions, craft_sync::versions::MIN_GAP)
    {
        log::warn!("couldn't keep an earlier version of {path}: {e}");
    }
}

/// Mirror the shared AI folder with the cloud folder now (the thread `craft_sync::background`
/// runs, started with the assistant's memory, does it every few minutes anyway).
pub fn sync_now() {
    craft_sync::background::sync_now();
}

/// Every frame: the open workbook's cloud copies when another workbook comes on screen, and a
/// version or copy chosen on the settings page.
pub(crate) fn tick(app: &mut SheetApp) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    if let Some(p) = with(|s| s.open.take()) {
        app.open_path(&p.to_string_lossy());
    }
    let path = app.session.active().and_then(|d| d.path.clone());
    let changed = with(|s| {
        let changed = s.checked != path;
        if changed {
            s.checked.clone_from(&path);
        }
        changed
    });
    if changed
        && app.notice.is_none()
        && let Some(p) = path
    {
        let copies = craft_sync::cloud::conflict_copies(Path::new(&p));
        if !copies.is_empty() {
            let names: Vec<String> = copies.iter().filter_map(|c| c.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            app.notice = Some(crate::Notice {
                text: format!(
                    "Your cloud folder also kept {}: two computers changed this workbook before they caught up. Compare them from File › Options › Save.",
                    names.join(", ")
                ),
                kind: crate::NoticeKind::Warning,
                action: None,
            });
        }
    }
}

fn hint(ui: &mut Ui, text: &str) {
    let dim = Tokens::get(ui.ctx()).text_dim;
    ui.label(RichText::new(text).font(theme::ui_font(11.5)).color(dim));
}

fn title(ui: &mut Ui, text: &str) {
    let colour = Tokens::get(ui.ctx()).text;
    ui.add_space(14.0);
    ui.label(RichText::new(text).font(theme::ui_bold(15.0)).color(colour));
    ui.add_space(8.0);
}

/// File › Options › Save: syncing, earlier versions, and this workbook's versions and copies.
pub(crate) fn options(app: &mut SheetApp, ui: &mut Ui) {
    if cfg!(target_arch = "wasm32") {
        title(ui, "Sync");
        hint(ui, "Syncing and earlier versions work in the desktop app.");
        return;
    }
    let mut s = settings();
    let before = s.clone();
    sync_settings(ui, &mut s, before.enabled);
    version_settings(ui, &mut s);
    if s != before {
        let saved = s.save();
        let turned_on = s.enabled && !before.enabled;
        with(|st| st.settings = s.clone());
        match saved {
            Err(e) => app.message = Some(("GridCraft".into(), format!("Couldn't save the sync settings: {e}"))),
            Ok(()) if turned_on => sync_now(),
            Ok(()) => {}
        }
    }
    if let Some(path) = app.session.active().and_then(|d| d.path.clone()) {
        this_file(ui, &s, Path::new(&path));
    }
}

/// On or off, the folder, Sync now. `was_on`: syncing as saved (Sync now syncs that).
fn sync_settings(ui: &mut Ui, s: &mut Settings, was_on: bool) {
    let dim = Tokens::get(ui.ctx()).text_dim;
    title(ui, "Sync with iCloud Drive");
    ui.checkbox(&mut s.enabled, "Keep AI conversations and shared settings in a cloud folder");
    hint(
        ui,
        "Every Craft app on each of your computers uses the same folder, so a file's conversation is there when you open the file on another computer. Keys for your own AI accounts stay on this computer.",
    );
    let shown = s.folder().map(|p| p.display().to_string()).unwrap_or_default();
    let mut text = with(|st| st.folder_text.clone()).unwrap_or(shown);
    ui.horizontal(|ui| {
        ui.label("Folder");
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(380.0).hint_text("No cloud folder found"));
        if r.changed() {
            with(|st| st.folder_text = Some(text.clone()));
        }
        if r.lost_focus() {
            let typed = text.trim();
            s.folder = (!typed.is_empty()).then(|| PathBuf::from(typed));
            with(|st| st.folder_text = None);
        }
    });
    let roots = craft_sync::cloud::roots();
    if roots.is_empty() {
        hint(
            ui,
            "No iCloud Drive, OneDrive, Google Drive or Dropbox folder was found on this computer: install iCloud for Windows, or type a folder.",
        );
    }
    ui.horizontal_wrapped(|ui| {
        for r in &roots {
            let folder = r.path.join("ArtCraft");
            if ui.small_button(format!("Use {}", r.kind.label())).on_hover_text(folder.display().to_string()).clicked() {
                s.folder = Some(folder);
                with(|st| st.folder_text = None);
            }
        }
    });
    ui.horizontal(|ui| {
        let running = craft_sync::background::running();
        if ui.add_enabled(was_on && !running, egui::Button::new("Sync now")).clicked() {
            sync_now();
        }
        let status = if running { "Syncing…".to_string() } else { craft_sync::background::report().unwrap_or_default() };
        ui.label(RichText::new(status).color(dim));
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    });
}

/// How many versions to keep, and where they go.
fn version_settings(ui: &mut Ui, s: &mut Settings) {
    title(ui, "Earlier versions");
    ui.horizontal(|ui| {
        ui.label("Versions kept of each workbook");
        ui.add(egui::DragValue::new(&mut s.keep_versions).range(0..=craft_sync::settings::MAX_VERSIONS));
    });
    hint(ui, "Before a save replaces a workbook, the workbook as it was is kept (AutoSave keeps one at most every ten minutes). 0 keeps none.");
    if let Some(dir) = s.versions_dir() {
        hint(ui, &format!("Kept in {}", dir.display()));
    }
}

/// The open workbook's earlier versions and the copies the cloud kept of it, each with Open.
fn this_file(ui: &mut Ui, s: &Settings, file: &Path) {
    let versions = s.versions_dir().map(|root| craft_sync::versions::list(&root, file)).unwrap_or_default();
    ui.add_space(8.0);
    if versions.is_empty() {
        hint(ui, "This workbook has no earlier versions yet.");
    } else {
        ui.label(RichText::new("This workbook's earlier versions").strong());
    }
    for v in versions.iter().take(MAX_LISTED) {
        open_row(ui, &v.label(), &v.path, "Opens this version as a workbook of its own: use Save As to keep it.");
    }
    let copies = craft_sync::cloud::conflict_copies(file);
    if copies.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Copies your cloud folder kept").strong());
    hint(
        ui,
        "Two computers changed this workbook before they caught up, so the cloud kept both. Open one to compare, keep what you need, then delete the other.",
    );
    for c in &copies {
        let name = c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        open_row(ui, &name, c, "Opens this copy as a workbook of its own.");
    }
}

/// A line with Open: the file opens on the next frame (and the File view closes).
fn open_row(ui: &mut Ui, label: &str, file: &Path, tip: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.small_button("Open").on_hover_text(tip).clicked() {
            with(|st| st.open = Some(file.to_path_buf()));
        }
    });
}
