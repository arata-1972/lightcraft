//! The import review dialog (File → Import Photos… / Import from Folder… / Import from Device):
//! the source it scanned (a scanned folder is *not* added to Local), the files found under it as
//! a grid of thumbnails with checkboxes (duplicates marked and unchecked), the destination (add in
//! place / copy or move into the library's `Originals/` or a chosen folder, filed by day, by month,
//! into one folder or by a custom folder template, optionally renamed), an album
//! (existing or new), a preset and keywords to apply. Importing runs in small batches, one per
//! frame, with a progress window; the whole import is one undo step.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_engine::import::{ImportCandidate, ScanInput, ScanOutput, ScanProgress, scan_with};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

use crate::LightcraftApp;
use crate::render::Slot;
use crate::theme::Tokens;
use crate::widgets::register;

/// Files per batch (one batch per frame, so the progress window updates).
pub(crate) const BATCH: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportDialog {
    /// The files / folders that were scanned (shown as the source).
    pub sources: Vec<String>,
    pub candidates: Vec<ImportCandidate>,
    pub checked: Vec<bool>,
    /// Copy into the library (else add in place).
    pub copy: bool,
    /// With `copy`: move instead — the originals are removed from the source once each copy is
    /// verified and catalogued (`library.import` mode `move`).
    pub move_files: bool,
    /// Existing album to add to.
    pub album: Option<u64>,
    /// …or a new album with this name.
    pub new_album: String,
    /// Preset id ("" = none).
    pub preset: String,
    /// Comma-separated keywords.
    pub keywords: String,
    /// Copy: destination folder ("" = the library's Originals/).
    pub destination: String,
    /// Copy: `date` (YYYY/YYYY-MM-DD), `month`, `flat` or `custom` ([`ImportDialog::folder_template`]).
    pub organize: String,
    /// Copy, `custom`: the folder template, e.g. `{date:%Y}/{date:%Y%m%d}`.
    pub folder_template: String,
    /// Copy: file-name template for the copies ("" = keep the names).
    pub rename: String,
    /// Metadata preset name ("" = none).
    pub metadata_preset: String,
    /// Copy: raws are copied as DNG.
    pub dng: bool,
}

impl ImportDialog {
    pub fn new(candidates: Vec<ImportCandidate>) -> Self {
        let checked = candidates.iter().map(|c| c.duplicate.is_none() && c.error.is_none()).collect();
        ImportDialog { candidates, checked, ..Default::default() }
    }
    pub fn importable(&self, i: usize) -> bool {
        self.candidates.get(i).is_some_and(|c| c.duplicate.is_none() && c.error.is_none())
    }
    pub fn selected_paths(&self) -> Vec<String> {
        self.candidates
            .iter()
            .zip(&self.checked)
            .filter(|(c, on)| **on && c.duplicate.is_none() && c.error.is_none())
            .map(|(c, _)| c.path.clone())
            .collect()
    }
    /// The `organize` param of `library.import` (`None` = the default, by day); an unusable
    /// custom folder template is an error.
    pub fn organize_param(&self) -> Result<Option<String>, String> {
        match self.organize.as_str() {
            "" => Ok(None),
            "custom" => {
                let t = self.folder_template.trim();
                if let Some(e) = lightcraft_engine::rename::folder_template_error(t) {
                    return Err(e);
                }
                // a plain folder name ("Imports") is a one-level template too
                Ok(Some(if t.contains(['{', '/', '\\']) { t.to_string() } else { format!("{t}/") }))
            }
            o => Ok(Some(o.to_string())),
        }
    }
    fn keywords(&self) -> Vec<String> {
        self.keywords.split(',').map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).collect()
    }
}

/// A running import (see the module docs).
#[derive(Debug, Default)]
pub struct ImportTask {
    queue: Vec<String>,
    pub total: usize,
    pub done: usize,
    params: Value,
    pub imported: usize,
    pub duplicates: usize,
    pub failed: usize,
    /// Move: originals moved, and sources left in place (reported by the engine with a reason).
    pub moved: usize,
    pub kept: usize,
    undo0: usize,
    first: Option<u64>,
    /// Reading a folder for the Local view: the photos stay out of the library, and nothing is
    /// selected or announced as added.
    browse: bool,
}

/// A folder scan running on a worker thread (a network share can take minutes to read; the
/// window must keep answering meanwhile).
pub struct ScanTask {
    progress: std::sync::Arc<ScanProgress>,
    rx: std::sync::mpsc::Receiver<ScanOutput>,
    /// Open the review with "copy into the library" checked (a camera / card).
    pub copy: bool,
    /// Browsing a folder (Local): the photos are read in place instead of opening the review.
    browse: bool,
    /// What is being scanned (the review's source).
    sources: Vec<String>,
}

/// Scan `paths` in the background, then open the review dialog (see [`poll_scan`]).
pub fn open(app: &mut LightcraftApp, paths: Vec<String>) -> Result<Value, String> {
    if app.scan.is_some() {
        return Err("a scan is already running".into());
    }
    let sources = paths.clone();
    let (input, paths) = ScanInput::new(&mut app.session, &paths);
    let progress = std::sync::Arc::new(ScanProgress::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let p = progress.clone();
    let job = move || {
        let _ = tx.send(scan_with(input, &paths, &p));
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(job);
    #[cfg(target_arch = "wasm32")]
    job();
    app.scan = Some(ScanTask { progress, rx, copy: false, browse: false, sources });
    Ok(json!({"scanning": true}))
}

/// Show a folder's photos in the Local view (`library.browse` from the UI): the view switches at
/// once, the folder is listed and read on a worker thread (a network share can take minutes),
/// and the photos then join the view in small batches under a progress window. A browse already
/// running is replaced; the import review's scan is not.
pub fn browse(app: &mut LightcraftApp, path: &str, subfolders: Option<bool>) -> Result<Value, String> {
    let dir = std::path::absolute(std::path::Path::new(path)).map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        return Err(format!("{path}: not a folder"));
    }
    if app.scan.as_ref().is_some_and(|t| !t.browse) || app.import.as_ref().is_some_and(|t| !t.browse) {
        return Err("an import is running".into());
    }
    let dir_s = dir.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
    let subfolders = subfolders.unwrap_or_else(|| app.session.browse.as_ref().is_some_and(|b| b.subfolders));
    let running = app.scan.as_ref().is_some_and(|t| t.browse) || app.import.as_ref().is_some_and(|t| t.browse);
    if running && app.session.browse.as_ref().is_some_and(|b| b.path == dir_s && b.subfolders == subfolders) {
        // already reading this folder: clicking it again must not restart the progress
        app.session.source = lightcraft_engine::LibrarySource::Folder;
        return Ok(json!({"path": dir_s, "subfolders": subfolders, "scanning": true}));
    }
    if let Some(t) = app.scan.take() {
        t.progress.cancel.store(true, Ordering::Relaxed);
    }
    app.import = None;
    app.session.browse = Some(lightcraft_engine::Browse { path: dir_s.clone(), subfolders });
    app.session.source = lightcraft_engine::LibrarySource::Folder;
    let (input, _) = ScanInput::new(&mut app.session, std::slice::from_ref(&dir_s));
    let progress = std::sync::Arc::new(ScanProgress::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let p = progress.clone();
    let root = dir_s.clone();
    let job = move || {
        let files: Vec<String> = if subfolders {
            lightcraft_engine::import::expand(&[root], None)
        } else {
            let mut v: Vec<String> = std::fs::read_dir(&root)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|f| {
                            f.is_file()
                                && lightcraft_engine::import::is_supported(f)
                                && !f.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
                        })
                        .map(|f| f.to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        };
        let _ = tx.send(scan_with(input, &files, &p));
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(job);
    #[cfg(target_arch = "wasm32")]
    job();
    app.scan = Some(ScanTask { progress, rx, copy: false, browse: true, sources: Vec::new() });
    app.renderer.forget_imports();
    Ok(json!({"path": dir_s, "subfolders": subfolders, "scanning": true}))
}

/// Collect a finished scan and open the review (called every frame).
pub fn poll_scan(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = app.scan.as_ref() else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let out = match task.rx.try_recv() {
        Ok(o) => o,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(_) => {
            app.scan = None;
            app.toast(ctx, "Scan failed");
            return;
        }
    };
    let Some(task) = app.scan.take() else { return };
    if task.progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    app.session.import_probes = out.probes;
    if task.browse {
        // what the library doesn't know yet joins the Local view
        let queue: Vec<String> =
            out.candidates.iter().filter(|c| c.duplicate != Some("path".into()) && c.error.is_none()).map(|c| c.path.clone()).collect();
        if !queue.is_empty() {
            let undo0 = app.session.undo.len();
            let total = queue.len();
            app.import = Some(ImportTask { queue, total, params: json!({"mode": "add", "local": true}), undo0, browse: true, ..Default::default() });
        }
        return;
    }
    if out.candidates.is_empty() {
        app.toast(ctx, "No photos found");
        return;
    }
    app.renderer.forget_imports();
    let mut d = ImportDialog::new(out.candidates);
    d.copy = task.copy;
    d.sources = task.sources;
    app.ui.dialog = Some(crate::state::Dialog::Import { opts: Box::new(d) });
}

/// The progress window while a folder is being scanned.
pub fn scan_progress(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = &app.scan else { return };
    let t = Tokens::get(ctx);
    let total = task.progress.total.load(Ordering::Relaxed);
    let done = task.progress.done.load(Ordering::Relaxed);
    let text = if total == 0 {
        if task.browse { "Reading folder…" } else { "Looking for photos…" }.to_string()
    } else {
        crate::i18n::tr_format!("Reading photos… {done} of {total}", done = done, total = total)
    };
    let mut cancel = false;
    egui::Window::new(crate::i18n::tr("Scanning"))
        .title_bar(false)
        .resizable(false)
        .anchor(Align2::CENTER_BOTTOM, [0.0, -80.0])
        .fixed_size([340.0, 80.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(text).color(t.text));
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(320.0));
            let r = ui.button(crate::i18n::tr("Cancel"));
            register(ui.ctx(), "button:scanCancel", r.rect);
            cancel = r.clicked();
        });
    if cancel {
        // the worker stops at its next file; don't wait for it (a NAS read can take a while)
        task.progress.cancel.store(true, Ordering::Relaxed);
        app.scan = None;
    }
}

impl ScanTask {
    /// `{done, total}` for `ui.inspect` (total is 0 while the folders are still being listed).
    pub fn status(&self) -> Value {
        json!({"done": self.progress.done.load(Ordering::Relaxed), "total": self.progress.total.load(Ordering::Relaxed)})
    }
}

/// Start importing the dialog's checked files (the dialog's OK / `ui.dialog.confirm`).
pub fn start(app: &mut LightcraftApp, d: &ImportDialog) -> Result<Value, String> {
    let queue = d.selected_paths();
    if queue.is_empty() {
        return Err("no photos selected".into());
    }
    let undo0 = app.session.undo.len();
    let mut album = d.album;
    if album.is_none() && !d.new_album.trim().is_empty() {
        let r = app.session.execute("album.create", &json!({"name": d.new_album.trim()})).map_err(|e| e.to_string())?;
        album = r["id"].as_u64();
    }
    let mode = match (d.copy, d.move_files) {
        (false, _) => "add",
        (true, false) => "copy",
        (true, true) => "move",
    };
    let mut params = json!({"mode": mode, "keywords": d.keywords()});
    if let Some(a) = album {
        params["album"] = json!(a);
    }
    if !d.preset.is_empty() {
        params["preset"] = json!(d.preset);
    }
    if !d.metadata_preset.is_empty() {
        params["metadataPreset"] = json!(d.metadata_preset);
    }
    if d.copy {
        if !d.destination.trim().is_empty() {
            params["destination"] = json!(d.destination.trim());
        }
        if let Some(o) = d.organize_param()? {
            params["organize"] = json!(o);
        }
        if !d.rename.trim().is_empty() {
            params["rename"] = json!(d.rename.trim());
            params["renameStart"] = json!(1);
        }
        if d.dng && !d.move_files {
            params["dng"] = json!(true);
        }
    }
    let total = queue.len();
    app.import = Some(ImportTask { queue, total, params, undo0, ..Default::default() });
    app.renderer.forget_imports();
    Ok(json!({"importing": total}))
}

/// Run one batch of the import in progress (called every frame).
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = app.import.as_mut() else { return };
    // (browsing reads a sidecar per file, slow on a network share: keep each frame short)
    let n = task.queue.len().min(if task.browse { 2 } else { BATCH });
    let batch: Vec<String> = task.queue.drain(..n).collect();
    let mut p = task.params.clone();
    p["paths"] = json!(batch);
    // renamed copies keep counting across batches
    if p.get("rename").is_some() {
        p["renameStart"] = json!(1 + task.imported);
    }
    let r = app.session.execute("library.import", &p);
    let Some(task) = app.import.as_mut() else { return };
    task.done += n;
    match r {
        Ok(v) => {
            let len = |k: &str| v[k].as_array().map_or(0, Vec::len);
            task.imported += len("imported");
            task.duplicates += len("duplicates");
            task.failed += len("failed");
            task.moved += len("moved");
            task.kept += len("kept");
            for k in v["kept"].as_array().into_iter().flatten() {
                log::warn!("import: kept {}: {}", k["path"].as_str().unwrap_or(""), k["reason"].as_str().unwrap_or(""));
            }
            if task.first.is_none() {
                task.first = v["imported"].as_array().and_then(|a| a.first()).and_then(Value::as_u64);
            }
        }
        Err(e) => {
            log::warn!("import: {e}");
            task.failed += n;
        }
    }
    ctx.request_repaint();
    if !task.queue.is_empty() {
        return;
    }
    let Some(task) = app.import.take() else { return };
    let steps = app.session.undo.len().saturating_sub(task.undo0);
    let label = crate::i18n::tr_format!("Add {} Photo{}", task.imported, if task.imported == 1 { "" } else { "s" });
    app.session.merge_undo(steps, &label);
    if task.browse {
        if task.failed > 0 {
            app.toast(ctx, crate::i18n::tr_format!("{} photo{} not readable", task.failed, if task.failed == 1 { "" } else { "s" }));
        }
        return;
    }
    if let Some(f) = task.first {
        let _ = app.run("library.select", json!({"ids": [f]}));
    }
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut msg = crate::i18n::tr_format!("Imported {} photo{}", task.imported, plural(task.imported));
    if task.params["mode"] == "move" {
        msg.push_str(&crate::i18n::tr_format!(" · {} moved", task.moved));
        if task.kept > 0 {
            msg.push_str(&format!(" · {} original{} left at the source", task.kept, plural(task.kept)));
        }
    }
    if task.duplicates > 0 {
        msg.push_str(&crate::i18n::tr_format!(" · {} duplicate{} skipped", task.duplicates, if task.duplicates == 1 { "" } else { "s" }));
    }
    if task.failed > 0 {
        msg.push_str(&crate::i18n::tr_format!(" · {} not readable", task.failed));
    }
    app.toast(ctx, msg);
}

/// The progress window while an import runs.
pub fn progress(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = &app.import else { return };
    let t = Tokens::get(ctx);
    let frac = task.done as f32 / task.total.max(1) as f32;
    let text = format!("{} photos… {} of {}", if task.browse { "Reading" } else { "Adding" }, task.done, task.total);
    egui::Window::new(crate::i18n::tr("Importing"))
        .title_bar(false)
        .resizable(false)
        .anchor(Align2::CENTER_BOTTOM, [0.0, -80.0])
        .fixed_size([340.0, 60.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(text).color(t.text));
            ui.add(egui::ProgressBar::new(frac).desired_width(320.0));
        });
}

/// The dialog body: options, then the candidate grid.
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut ImportDialog) {
    let t = Tokens::get(ui.ctx());
    let n = d.candidates.len();
    let dups = d.candidates.iter().filter(|c| c.duplicate.is_some()).count();
    let sel = d.selected_paths().len();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::tr_format!("{n} found · {sel} selected", n = n, sel = sel)).color(t.text));
        if dups > 0 {
            ui.label(egui::RichText::new(crate::i18n::tr_format!("· {dups} already in the library", dups = dups)).color(t.text_dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::text_button(ui, "importNone", crate::i18n::tr("Uncheck All"), false).clicked() {
                d.checked.iter_mut().for_each(|c| *c = false);
            }
            if crate::widgets::text_button(ui, "importAll", crate::i18n::tr("Check All"), false).clicked() {
                for i in 0..n {
                    d.checked[i] = d.importable(i);
                }
            }
        });
    });
    // candidate grid
    let cell = 116.0;
    let avail = ui.available_width();
    let cols = ((avail + 6.0) / (cell + 6.0)).floor().max(1.0) as usize;
    let rows = n.div_ceil(cols);
    egui::ScrollArea::vertical().id_salt("import-grid").max_height(330.0).auto_shrink([false, true]).show_viewport(ui, |ui, viewport| {
        let (area, _) = ui.allocate_exact_size(vec2(avail, rows as f32 * (cell + 26.0)), Sense::hover());
        for i in 0..n {
            let (c, r) = (i % cols, i / cols);
            let local = Rect::from_min_size(pos2(c as f32 * (cell + 6.0), r as f32 * (cell + 26.0)), vec2(cell, cell + 20.0));
            if !local.intersects(viewport.expand(cell)) {
                continue;
            }
            let rect = local.translate(area.min.to_vec2());
            candidate_cell(app, ui, d, i, rect);
        }
    });
    ui.add_space(4.0);
    // options
    if !d.sources.is_empty() {
        field(ui, "Source", |ui| {
            let r = ui.label(egui::RichText::new(source_summary(&d.sources)).color(t.text_label)).on_hover_text(d.sources.join("\n"));
            register(ui.ctx(), "label:importSource", r.rect);
        });
        ui.label(
            egui::RichText::new(crate::i18n::tr("Importing scans the source for photos; it doesn't add the folder to Local (use Local → Browse Folder… to work in a folder without importing)."))
                .color(t.text_dim)
                .small(),
        );
    }
    field(ui, "Transfer", |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if crate::widgets::text_button(ui, "importAdd", crate::i18n::tr("Add in place"), !d.copy)
            .on_hover_text(crate::i18n::tr("Reference the files where they are"))
            .clicked()
        {
            d.copy = false;
            d.move_files = false;
        }
        let can_copy = app.session.library.as_ref().is_some_and(|l| l.on_disk) || app.services.pick_folder.is_some();
        let r =
            ui.add_enabled_ui(can_copy, |ui| crate::widgets::text_button(ui, "importCopy", crate::i18n::tr("Copy"), d.copy && !d.move_files)).inner;
        if r.on_hover_text(crate::i18n::tr("Copy the files (into the library's Originals/, or a folder you choose)")).clicked() {
            d.copy = true;
            d.move_files = false;
        }
        let r =
            ui.add_enabled_ui(can_copy, |ui| crate::widgets::text_button(ui, "importMove", crate::i18n::tr("Move"), d.copy && d.move_files)).inner;
        if r.on_hover_text(crate::i18n::tr("Move the files (into the library's Originals/, or a folder you choose), removing them from the source"))
            .clicked()
        {
            d.copy = true;
            d.move_files = true;
        }
    });
    let r = ui.label(
        egui::RichText::new(match (d.copy, d.move_files) {
            (true, false) => {
                "Copy: the files are copied to the folder below and the library uses the copies; the originals are left as they are."
            }
            (true, true) => {
                "Move: the files are moved to the folder below; each original (and its XMP sidecar) is removed from the source only after its copy is verified. Duplicates and files that fail stay where they are."
            }
            _ => "Add in place: the library references the files where they are; nothing is copied or moved.",
        })
        .color(if d.copy && d.move_files { t.caution } else { t.text_dim })
        .small(),
    );
    register(ui.ctx(), "label:importModeHelp", r.rect);
    if d.copy {
        field(ui, if d.move_files { "Move to" } else { "Copy to" }, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let shown = if d.destination.trim().is_empty() { "Library Originals".to_string() } else { d.destination.clone() };
            ui.label(egui::RichText::new(shown).color(Tokens::get(ui.ctx()).text_label));
            if app.services.pick_folder.is_some()
                && crate::widgets::text_button(ui, "importDest", crate::i18n::tr("Choose…"), false).clicked()
                && let Some(f) = app.services.pick_folder.as_mut().and_then(|f| f())
            {
                d.destination = f;
            }
            if !d.destination.is_empty() && crate::widgets::text_button(ui, "importDestReset", crate::i18n::tr("Library"), false).clicked() {
                d.destination.clear();
            }
        });
        field(ui, "Folders", |ui| {
            let opts = [
                ("date", "By day (YYYY/YYYY-MM-DD)"),
                ("month", "By month (YYYY/YYYY-MM)"),
                ("flat", "Into one folder"),
                ("custom", "Custom template…"),
            ];
            let key = if d.organize.is_empty() { "date".to_string() } else { d.organize.clone() };
            let cur = opts.iter().find(|o| o.0 == key).map_or(opts[0].1, |o| o.1);
            let combo = egui::ComboBox::from_id_salt("import-organize").selected_text(cur).show_ui(ui, |ui| {
                for (k, label) in opts {
                    let r = ui.selectable_label(key == k, label);
                    register(ui.ctx(), format!("button:importOrganize-{k}"), r.rect);
                    if r.clicked() {
                        d.organize = k.to_string();
                        if k == "custom" && d.folder_template.trim().is_empty() {
                            d.folder_template = DEFAULT_FOLDER_TEMPLATE.into();
                        }
                    }
                }
            });
            register(ui.ctx(), "combo:importOrganize", combo.response.rect);
        });
        if d.organize == "custom" {
            let folders_id = egui::Id::new("import-folder-template");
            let tags_open = field(ui, "Template", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let w = (ui.available_width() - 50.0).max(80.0);
                let r = ui.add(egui::TextEdit::singleline(&mut d.folder_template).id(folders_id).hint_text(DEFAULT_FOLDER_TEMPLATE).desired_width(w));
                register(ui.ctx(), "field:importFolderTemplate", r.rect);
                tag_toggle(ui, "importFolders")
            });
            if tags_open {
                tag_help(ui, "importFolders", &mut d.folder_template, folders_id);
            }
            let t = Tokens::get(ui.ctx());
            match lightcraft_engine::rename::folder_template_error(&d.folder_template) {
                Some(e) => {
                    ui.label(egui::RichText::new(e).color(t.caution));
                }
                None => unknown_tags_warning(ui, &d.folder_template),
            }
            ui.label(
                egui::RichText::new(crate::i18n::tr(
                    "Each / starts a folder level; tags are filled in per photo (a level with missing metadata is \"unknown\").",
                ))
                .color(t.text_dim)
                .small(),
            );
        }
        if !d.move_files {
            field(ui, "Raw files", |ui| {
                let r = ui.checkbox(&mut d.dng, crate::i18n::tr("Copy as DNG"));
                register(ui.ctx(), "check:importDng", r.rect);
            });
        }
        let rename_id = egui::Id::new("import-rename");
        let tags_open = field(ui, "Rename", |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let w = (ui.available_width() - 50.0).max(80.0);
            let r = ui.add(egui::TextEdit::singleline(&mut d.rename).id(rename_id).hint_text("keep names — or e.g. {date}_{seq:3}").desired_width(w));
            register(ui.ctx(), "field:importRename", r.rect);
            tag_toggle(ui, "importRename")
        });
        if tags_open {
            tag_help(ui, "importRename", &mut d.rename, rename_id);
        }
        unknown_tags_warning(ui, &d.rename);
        if let Some(example) = example_destination(app, d) {
            let t = Tokens::get(ui.ctx());
            let r = ui.label(egui::RichText::new(crate::i18n::tr_format!("Example: {example}", example = example)).color(t.text_dim));
            register(ui.ctx(), "label:importExample", r.rect);
            ui.label(
                egui::RichText::new(crate::i18n::tr("Folders and {date} use the capture time; a photo without one uses today's date."))
                    .color(t.text_dim)
                    .small(),
            );
        }
    }
    let albums: Vec<(u64, String)> = {
        let mut v: Vec<(u64, String)> =
            app.session.catalog.albums().filter(|a| !a.folder && !a.is_smart()).map(|a| (a.id.0, a.name.clone())).collect();
        v.sort_by_key(|(_, n)| n.to_lowercase());
        v
    };
    field(ui, "Album", |ui| {
        let cur = match d.album {
            Some(a) => albums.iter().find(|x| x.0 == a).map(|x| x.1.clone()).unwrap_or_default(),
            None if !d.new_album.is_empty() => "New album".into(),
            None => "None".into(),
        };
        egui::ComboBox::from_id_salt("import-album").selected_text(cur).show_ui(ui, |ui| {
            if ui.selectable_label(d.album.is_none() && d.new_album.is_empty(), crate::i18n::tr("None")).clicked() {
                d.album = None;
                d.new_album.clear();
            }
            if ui.selectable_label(d.album.is_none() && !d.new_album.is_empty(), crate::i18n::tr("New album…")).clicked() {
                d.album = None;
                if d.new_album.is_empty() {
                    d.new_album = "Imported Photos".into();
                }
            }
            for (id, name) in &albums {
                if ui.selectable_label(d.album == Some(*id), name).clicked() {
                    d.album = Some(*id);
                    d.new_album.clear();
                }
            }
        });
        if d.album.is_none() && !d.new_album.is_empty() {
            let r = ui.add(egui::TextEdit::singleline(&mut d.new_album).desired_width(f32::INFINITY));
            register(ui.ctx(), "field:importAlbumName", r.rect);
        }
    });
    field(ui, "Preset", |ui| {
        let cur = app.session.presets.iter().find(|p| p.id == d.preset).map(|p| p.name.clone()).unwrap_or_else(|| "None".into());
        egui::ComboBox::from_id_salt("import-preset").selected_text(cur).height(300.0).show_ui(ui, |ui| {
            if ui.selectable_label(d.preset.is_empty(), crate::i18n::tr("None")).clicked() {
                d.preset.clear();
            }
            for p in &app.session.presets {
                if ui.selectable_label(d.preset == p.id, format!("{} — {}", p.group, p.name)).clicked() {
                    d.preset = p.id.clone();
                }
            }
        });
    });
    if !app.session.metadata_presets.is_empty() {
        field(ui, "Metadata", |ui| {
            let cur = if d.metadata_preset.is_empty() { "None".to_string() } else { d.metadata_preset.clone() };
            egui::ComboBox::from_id_salt("import-metadata").selected_text(cur).show_ui(ui, |ui| {
                if ui.selectable_label(d.metadata_preset.is_empty(), crate::i18n::tr("None")).clicked() {
                    d.metadata_preset.clear();
                }
                for m in &app.session.metadata_presets {
                    if ui.selectable_label(d.metadata_preset == m.name, &m.name).clicked() {
                        d.metadata_preset = m.name.clone();
                    }
                }
            });
        });
    }
    field(ui, "Keywords", |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut d.keywords).hint_text(crate::i18n::tr("comma, separated")).desired_width(f32::INFINITY));
        register(ui.ctx(), "field:importKeywords", r.rect);
    });
}

fn candidate_cell(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut ImportDialog, i: usize, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let c = d.candidates[i].clone();
    let ok = d.importable(i);
    let img = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width()));
    let resp = ui.interact(img, egui::Id::new(("import-cell", i)), Sense::click());
    register(ui.ctx(), format!("import:{i}"), img);
    let p = ui.painter();
    p.rect_filled(img, 3.0, Color32::from_gray(30));
    // thumbnail (background job; the grid only asks for the cells in view)
    let slot = Slot::Import(i as u32);
    if !app.renderer.textures.contains_key(&slot)
        && let Some(job) = app.session.candidate_thumb_job(&c, 192, i as u64)
    {
        app.renderer.request_quick(slot, job, 4);
    }
    if let Some(tex) = app.renderer.textures.get(&slot) {
        let [tw, th] = tex.size;
        let s = ((img.width() - 8.0) / tw as f32).min((img.height() - 8.0) / th as f32);
        let fit = Rect::from_center_size(img.center(), vec2(tw as f32 * s, th as f32 * s));
        let tint = if ok { Color32::WHITE } else { Color32::from_gray(110) };
        p.image(tex.tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
    let on = d.checked[i] && ok;
    if on {
        p.rect_stroke(img, 3.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    }
    // checkbox
    let cb = Rect::from_min_size(img.min + vec2(6.0, 6.0), vec2(16.0, 16.0));
    p.rect(cb, 3.0, if on { t.accent } else { Color32::from_black_alpha(150) }, Stroke::new(1.0, Color32::from_gray(200)), StrokeKind::Inside);
    if on {
        p.line_segment([cb.left_center() + vec2(3.5, 0.5), cb.center_bottom() + vec2(-1.0, -4.0)], Stroke::new(2.0, Color32::WHITE));
        p.line_segment([cb.center_bottom() + vec2(-1.0, -4.0), cb.right_top() + vec2(-3.5, 4.0)], Stroke::new(2.0, Color32::WHITE));
    }
    let badge = match (&c.duplicate, &c.error) {
        (Some(r), _) if r == "path" => Some("In library"),
        (Some(_), _) => Some("Duplicate"),
        (None, Some(_)) => Some("Unreadable"),
        _ => None,
    };
    if let Some(b) = badge {
        let g = p.layout_no_wrap(b.to_string(), t.semibold(10.0), Color32::WHITE);
        let br = Rect::from_min_size(pos2(img.right() - g.size().x - 12.0, img.top() + 6.0), g.size() + vec2(8.0, 4.0));
        p.rect_filled(br, 3.0, Color32::from_rgba_unmultiplied(170, 60, 50, 220));
        p.galley(br.min + vec2(4.0, 2.0), g, Color32::WHITE);
    }
    let name = if c.name.chars().count() > 18 { format!("{}…", c.name.chars().take(17).collect::<String>()) } else { c.name.clone() };
    p.text(pos2(rect.left() + 2.0, img.bottom() + 9.0), Align2::LEFT_CENTER, name, t.font(10.5), if ok { t.text_label } else { t.text_dim });
    let tip = format!(
        "{}\n{} × {} · {} · {:.1} MB{}",
        c.path,
        c.width,
        c.height,
        c.format,
        c.file_size as f64 / 1e6,
        c.captured.as_deref().map(|d| format!("\n{}", d.replace('T', " "))).unwrap_or_default()
    );
    let resp = resp.on_hover_text(tip);
    if resp.clicked() && ok {
        d.checked[i] = !d.checked[i];
    }
}

/// The "Tags" toggle beside a template field (`button:<key>Tags`); returns whether the tag help
/// ([`tag_help`]) is open.
pub(crate) fn tag_toggle(ui: &mut egui::Ui, key: &str) -> bool {
    let id = egui::Id::new((key, "tags-open"));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let r = crate::widgets::text_button(ui, &format!("{key}Tags"), crate::i18n::tr("Tags"), open)
        .on_hover_text(crate::i18n::tr("Show the template tags; click one to insert it"));
    if r.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }
    open
}

/// "Unknown tag {x} stays as typed" under a template field, when it has one.
pub(crate) fn unknown_tags_warning(ui: &mut egui::Ui, template: &str) {
    let unknown = lightcraft_engine::rename::unknown_tokens(template);
    if !unknown.is_empty() {
        let s = if unknown.len() == 1 { "" } else { "s" };
        let text = format!("Unknown tag{s} {} stay{} as typed (see Tags)", unknown.join(" "), if unknown.len() == 1 { "s" } else { "" });
        ui.label(egui::RichText::new(text).color(Tokens::get(ui.ctx()).caution));
    }
}

/// Insert `tag` into `text` at the text cursor of the field `edit_id` (replacing its selection);
/// appended when the field never had a cursor. The cursor ends up after the tag.
pub(crate) fn insert_at_cursor(ctx: &egui::Context, edit_id: egui::Id, text: &mut String, tag: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::text_edit::TextEditState::load(ctx, edit_id).unwrap_or_default();
    let n = text.chars().count();
    let (a, b) = state.cursor.char_range().map_or((n, n), |r| {
        let (x, y) = (r.primary.index.0.min(n), r.secondary.index.0.min(n));
        (x.min(y), x.max(y))
    });
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let (ba, bb) = (byte(a), byte(b));
    text.replace_range(ba..bb, tag);
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(a + tag.chars().count()))));
    state.store(ctx, edit_id);
    ctx.memory_mut(|m| m.request_focus(edit_id));
}

/// The template tag help: every tag (from the engine's one list, `rename::TOKENS`) with its
/// meaning and an example, the `{date:…}` directives and how templates behave. Clicking a tag
/// (`button:<key>Tag-<i>`) inserts it at the cursor of the field `edit_id`.
pub(crate) fn tag_help(ui: &mut egui::Ui, key: &str, text: &mut String, edit_id: egui::Id) {
    use lightcraft_engine::rename::{DATE_DIRECTIVES, TEMPLATE_NOTES, TOKENS, token_example};
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.inset).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
        ui.label(
            egui::RichText::new(crate::i18n::tr("Click a tag to insert it. Examples are for IMG_0042.CR3 taken 14 Jan 2026, 05:58:48."))
                .color(t.text_dim)
                .small(),
        );
        egui::ScrollArea::vertical().id_salt((key, "tags")).max_height(170.0).show(ui, |ui| {
            egui::Grid::new((key, "tag-grid")).num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
                for (i, tok) in TOKENS.iter().enumerate() {
                    let r = ui.add(egui::Button::new(egui::RichText::new(tok.tag).monospace().color(t.text)).small()).on_hover_text(
                        if tok.aliases.is_empty() {
                            "Insert at the cursor".to_string()
                        } else {
                            format!("Insert at the cursor (also written {})", tok.aliases.join(", "))
                        },
                    );
                    register(ui.ctx(), format!("button:{key}Tag-{i}"), r.rect);
                    if r.clicked() {
                        insert_at_cursor(ui.ctx(), edit_id, text, tok.tag);
                    }
                    ui.label(egui::RichText::new(tok.meaning).color(t.text_label));
                    ui.label(egui::RichText::new(token_example(tok.tag)).monospace().color(t.text_dim));
                    ui.end_row();
                }
            });
            let directives: Vec<String> = DATE_DIRECTIVES.iter().map(|(d, m)| format!("{d} {m}")).collect();
            ui.label(egui::RichText::new(format!("{{date:…}} directives: {}", directives.join(" · "))).color(t.text_dim).small());
            for n in TEMPLATE_NOTES {
                ui.label(egui::RichText::new(format!("• {n}")).color(t.text_dim).small());
            }
        });
    });
}

/// The folder template the Custom choice starts with (`2026/20260114/`).
pub const DEFAULT_FOLDER_TEMPLATE: &str = "{date:%Y}/{date:%Y%m%d}";

/// Where the first selected photo would be copied to (destination, folders, name), for the
/// dialog's example line; `None` without a photo or with an unusable folder template.
pub fn example_destination(app: &LightcraftApp, d: &ImportDialog) -> Option<String> {
    let c = d.candidates.iter().zip(&d.checked).find(|(c, on)| **on && c.duplicate.is_none() && c.error.is_none()).map(|(c, _)| c)?;
    let organize = match d.organize_param().ok()? {
        Some(o) => lightcraft_engine::import::Organize::parse(&o)?,
        None => Default::default(),
    };
    let mut q = lightcraft_catalog::Photo::new(
        lightcraft_catalog::PhotoId(0),
        lightcraft_catalog::Source::File { path: c.path.clone() },
        &c.name,
        &c.format,
        0,
        0,
        // a photo without a capture time is dated by the import (now)
        &(app.session.clock)(),
    );
    q.captured = c.captured.clone();
    // the probe's metadata, so {camera}, {title}… preview as they will import
    if let Some(info) = app.session.import_probes.get(&c.path) {
        q.meta = info.meta.clone();
    }
    let name = if d.rename.trim().is_empty() { c.name.clone() } else { lightcraft_engine::rename::expand(d.rename.trim(), &q, 1) };
    let root = if d.destination.trim().is_empty() {
        app.session.library.as_ref().map_or_else(|| "Originals".to_string(), |l| l.dir.join("Originals").to_string_lossy().to_string())
    } else {
        d.destination.trim().trim_end_matches(['/', '\\']).to_string()
    };
    let sep = std::path::MAIN_SEPARATOR_STR;
    let mut parts = vec![root];
    parts.extend(organize.folders(&q));
    parts.push(name);
    Some(parts.join(sep))
}

/// The import source in a few words: a folder's name, a file's name, or "N files and folders".
pub fn source_summary(sources: &[String]) -> String {
    let name = |p: &str| std::path::Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().to_string());
    match sources {
        [one] if std::path::Path::new(one).is_dir() => format!("Folder “{}” (and its subfolders)", name(one)),
        [one] => name(one),
        many => {
            let folders = many.iter().filter(|p| std::path::Path::new(p.as_str()).is_dir()).count();
            match folders {
                0 => crate::i18n::tr_format!("{} files", many.len()),
                f if f == many.len() => crate::i18n::tr_format!("{f} folders (and their subfolders)", f = f),
                f => crate::i18n::tr_format!("{} files and {f} folder{}", many.len() - f, if f == 1 { "" } else { "s" }, f = f),
            }
        }
    }
}

/// A labelled row (fixed label column).
fn field<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(78.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(78.0);
            ui.label(egui::RichText::new(label).color(t.text_label));
        });
        add(ui)
    })
    .inner
}
