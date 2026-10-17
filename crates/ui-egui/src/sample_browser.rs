use crate::{SoundApp, devices::BrowserItem};
use egui::{Sense, Ui};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{Receiver, TryRecvError}};

#[derive(Clone, Serialize, Deserialize)]
struct SampleEntry {
    path: String,
    name: String,
    search: String,
    bytes: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct SampleIndex {
    roots: Vec<String>,
    entries: Vec<SampleEntry>,
    directories: Vec<(String, Option<std::time::SystemTime>)>,
    errors: Vec<String>,
    truncated: bool,
}

struct PreviewRequest { pending: Option<(u64, String)>, stop: bool }

struct PreviewWorker {
    request: Arc<(std::sync::Mutex<PreviewRequest>, std::sync::Condvar)>,
    receiver: Receiver<(u64, String, Result<Arc<soundcraft_model::SourceAudio>, String>)>,
    generation: u64,
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.request;
        lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner).stop = true;
        wake.notify_one();
    }
}

impl PreviewWorker {
    fn new() -> Result<Self, String> {
        let request = Arc::new((std::sync::Mutex::new(PreviewRequest { pending: None, stop: false }), std::sync::Condvar::new()));
        let shared = Arc::clone(&request);
        let (sender, receiver) = std::sync::mpsc::sync_channel(2);
        std::thread::Builder::new().name("sample preview decoder".into()).spawn(move || loop {
            let job = {
                let (lock, wake) = &*shared;
                let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while state.pending.is_none() && !state.stop { state = wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner); }
                if state.stop { break; }
                state.pending.take()
            };
            let Some((generation, path)) = job else { continue };
            let result = std::panic::catch_unwind(|| decode_preview(&path)).unwrap_or_else(|_| Err("The sample decoder stopped unexpectedly. Try another file.".into()));
            if sender.send((generation, path, result)).is_err() { break; }
        }).map_err(|error| error.to_string())?;
        Ok(Self { request, receiver, generation: 0 })
    }

    fn request(&mut self, path: &str) {
        self.generation = self.generation.wrapping_add(1);
        let (lock, wake) = &*self.request;
        lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pending = Some((self.generation, path.to_string()));
        wake.notify_one();
    }
}

fn decode_preview(path: &str) -> Result<Arc<soundcraft_model::SourceAudio>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| format!("{path}: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > 134_217_728 { return Err("Preview supports sample files up to 128 MiB. Import the full file to play a larger recording.".into()); }
    let mut bytes = Vec::new();
    file.take(134_217_729).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 134_217_728 { return Err("The sample changed while it was being read; try again".into()); }
    let (_, audio) = soundcraft_audio_io::decode_preview(&bytes, Path::new(path).extension().and_then(|extension| extension.to_str()), 30).map_err(|error| error.to_string())?;
    Ok(Arc::new(soundcraft_model::SourceAudio::new(audio)))
}

pub struct SampleBrowser {
    initialized: bool,
    index: SampleIndex,
    scan: Option<(Receiver<SampleIndex>, Arc<AtomicBool>)>,
    preview: Option<PreviewWorker>,
    selected: String,
    view: String,
    root: String,
    filter_key: String,
    shown: Vec<usize>,
    gain_db: f32,
    focus: Option<egui::Id>,
    preview_waiting: bool,
}

impl Default for SampleBrowser {
    fn default() -> Self { Self { initialized: false, index: SampleIndex::default(), scan: None, preview: None, selected: String::new(), view: "Library".into(), root: String::new(), filter_key: String::new(), shown: Vec::new(), gain_db: -12.0, focus: None, preview_waiting: false } }
}

impl Drop for SampleBrowser {
    fn drop(&mut self) { if let Some((_, cancel)) = &self.scan { cancel.store(true, Ordering::Release); } }
}

fn scan(roots: Vec<String>, cancel: &AtomicBool) -> SampleIndex {
    let mut result = SampleIndex { roots: roots.clone(), ..SampleIndex::default() };
    let mut pending: Vec<_> = roots.into_iter().map(|path| (PathBuf::from(path), 0usize)).collect();
    let mut visited = HashSet::new();
    let mut files = HashSet::new();
    let mut budget = 500_000usize;
    while let Some((directory, depth)) = pending.pop() {
        if cancel.load(Ordering::Acquire) { break; }
        if budget == 0 || result.entries.len() >= 100_000 { result.truncated = true; break; }
        let key = std::fs::canonicalize(&directory).unwrap_or_else(|_| directory.clone());
        if !visited.insert(key) { continue; }
        let metadata = std::fs::metadata(&directory).ok();
        result.directories.push((directory.to_string_lossy().into_owned(), metadata.and_then(|metadata| metadata.modified().ok())));
        let entries = match std::fs::read_dir(&directory) { Ok(entries) => entries, Err(error) => { if result.errors.len() < 64 { result.errors.push(format!("{}: {error}", directory.display())); } continue; } };
        for entry in entries.flatten() {
            if cancel.load(Ordering::Acquire) { break; }
            if budget == 0 || result.entries.len() >= 100_000 { result.truncated = true; break; }
            budget -= 1;
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() { continue; }
            if kind.is_dir() {
                if depth < 24 { pending.push((path, depth + 1)); } else { result.truncated = true; }
            } else if kind.is_file() && crate::sampler_window::is_audio_name(&entry.file_name().to_string_lossy()) {
                let path = path.to_string_lossy().into_owned();
                if !files.insert(path.clone()) { continue; }
                let name = entry.file_name().to_string_lossy().into_owned();
                let bytes = entry.metadata().map_or(0, |metadata| metadata.len());
                let search = format!("{name} {path}").to_lowercase();
                result.entries.push(SampleEntry { path, name, search, bytes });
            }
        }
    }
    result.entries.sort_by(|a, b| a.search.cmp(&b.search));
    result
}

fn cache_path() -> Option<PathBuf> { crate::plugin_scan::cache_dir()?.parent().map(|directory| directory.join("sample-library-v1.json")) }

fn store(index: &SampleIndex) {
    let Some(path) = cache_path() else { return };
    if let Ok(bytes) = serde_json::to_vec(index) {
        if bytes.len() <= 33_554_432 {
            if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
            let staged = path.with_extension(format!("{}.partial", std::process::id()));
            if std::fs::write(&staged, bytes).is_ok() { let _ = std::fs::rename(staged, path); }
        }
    }
}

impl SampleBrowser {
    fn start(&mut self, roots: Vec<String>) {
        if let Some((_, cancel)) = self.scan.take() { cancel.store(true, Ordering::Release); }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = std::sync::mpsc::channel();
        match std::thread::Builder::new().name("sample library index".into()).spawn(move || { let result = scan(roots, &worker_cancel); if !worker_cancel.load(Ordering::Acquire) { let _ = sender.send(result); } }) {
            Ok(_) => self.scan = Some((receiver, cancel)),
            Err(error) => self.index.errors = vec![format!("Cannot start sample indexing: {error}. Choose Refresh to retry.")],
        }
    }

    fn initialize(&mut self, roots: &[String]) {
        if self.initialized { return; }
        self.initialized = true;
        if let Some(path) = cache_path() {
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.len() <= 33_554_432 {
                    if let Ok(bytes) = std::fs::read(path) {
                        if let Ok(index) = serde_json::from_slice::<SampleIndex>(&bytes) {
                            if index.roots == roots { self.index = index; }
                        }
                    }
                }
            }
        }
        let changed = self.index.roots != roots || self.index.directories.is_empty() || self.index.directories.iter().any(|(path, before)| std::fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok()) != *before);
        if !roots.is_empty() && changed { self.start(roots.to_vec()); }
    }

    fn preview(&mut self, app: &mut SoundApp, path: &str) {
        self.preview_waiting = false;
        if app.player.as_ref().is_none_or(|player| player.silent) { app.ui.status = "Choose an audio output to preview samples".into(); return; }
        if self.preview.is_none() {
            match PreviewWorker::new() { Ok(worker) => self.preview = Some(worker), Err(error) => { app.ui.status = error; return; } }
        }
        if let Some(player) = &app.player { player.stop_sample_preview(); }
        if let Some(worker) = &mut self.preview { worker.request(path); self.preview_waiting = true; app.ui.status = "Loading sample preview…".into(); }
    }

    fn poll(&mut self, app: &mut SoundApp) {
        match self.scan.as_ref().map(|(receiver, _)| receiver.try_recv()) {
            Some(Ok(index)) => { store(&index); self.index = index; self.filter_key.clear(); self.scan = None; }
            Some(Err(TryRecvError::Disconnected)) => { self.scan = None; app.ui.status = "The sample index stopped. Choose Refresh to retry.".into(); }
            _ => {}
        }
        let mut disconnected = false;
        if let Some(worker) = &self.preview {
            loop {
                let (generation, path, result) = match worker.receiver.try_recv() {
                    Ok(reply) => reply,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => { disconnected = true; break; }
                };
                if generation != worker.generation { continue; }
                self.preview_waiting = false;
                match result.and_then(|audio| app.player.as_ref().ok_or_else(|| "No audio output".to_string()).and_then(|player| { player.sample_preview_gain(soundcraft_dsp::db_to_gain(self.gain_db)); player.preview_sample(audio) })) {
                    Ok(()) => app.ui.status = format!("Preview: {}", Path::new(&path).file_name().unwrap_or_default().to_string_lossy()),
                    Err(error) => app.ui.status = error,
                }
            }
        }
        if disconnected {
            self.preview = None;
            self.preview_waiting = false;
            app.ui.status = "The sample preview worker stopped. Select a sample to retry.".into();
        }
    }

    fn import(&mut self, app: &mut SoundApp, path: &str) {
        if crate::devices::selected_track(app).and_then(|id| app.engine.session().track(id)).is_none_or(|track| !matches!(track.kind, soundcraft_model::TrackKind::Audio | soundcraft_model::TrackKind::Instrument)) {
            if let Ok(result) = app.run("track.new", serde_json::json!({"kind": "audio", "format": "stereo", "count": 1})) {
                if let Some(track) = result["tracks"][0].as_u64() { let _ = app.run("edit.select", serde_json::json!({"tracks": [track]})); }
            }
        }
        if let Some(track) = crate::devices::selected_track(app) {
            crate::devices::insert_item(app, track, &BrowserItem::Sample(path.to_string()));
            self.filter_key.clear();
        }
    }

    fn show(&mut self, app: &mut SoundApp, ui: &mut Ui) {
        self.initialize(&app.ui.sample_folders);
        self.poll(app);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add sample folder").clicked() {
                if let Some(path) = app.services.pick_folder.as_ref().and_then(|pick| pick("Add a sample library")) {
                    if !app.ui.sample_folders.contains(&path) {
                        if app.ui.sample_folders.len() < 64 { app.ui.sample_folders.push(path); }
                        else { app.ui.status = "The sample library supports up to 64 locations".into(); }
                    }
                    self.start(app.ui.sample_folders.clone());
                }
            }
            if ui.add_enabled(self.scan.is_none(), egui::Button::new("Refresh")).clicked() { self.start(app.ui.sample_folders.clone()); }
        });
        ui.horizontal_wrapped(|ui| { for view in ["Library", "Favorites", "Recent"] { if ui.selectable_value(&mut self.view, view.to_string(), view).clicked() { self.filter_key.clear(); } } });
        ui.horizontal(|ui| {
            ui.checkbox(&mut app.ui.auto_preview_samples, "Auto preview");
            if ui.add_enabled(!self.selected.is_empty(), egui::Button::new("Preview")).clicked() { self.preview(app, &self.selected.clone()); }
            if ui.small_button("Stop").clicked() { if let Some(player) = &app.player { player.stop_sample_preview(); } if let Some(worker) = &mut self.preview { worker.generation = worker.generation.wrapping_add(1); self.preview_waiting = false; } }
        });
        if ui.add(egui::Slider::new(&mut self.gain_db, -36.0..=0.0).text("Preview dB")).changed() { if let Some(player) = &app.player { player.sample_preview_gain(soundcraft_dsp::db_to_gain(self.gain_db)); } }
        egui::ComboBox::from_id_salt("sample_location").selected_text(if self.root.is_empty() { "All locations" } else { self.root.as_str() }).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.root, String::new(), "All locations");
            for root in &app.ui.sample_folders { ui.selectable_value(&mut self.root, root.clone(), root); }
        });
        if !self.root.is_empty() && ui.small_button("Remove this location").clicked() {
            app.ui.sample_folders.retain(|path| path != &self.root); self.root.clear(); self.start(app.ui.sample_folders.clone());
        }
        if self.scan.is_some() { ui.spinner(); ui.weak("Indexing samples in the background…"); ui.ctx().request_repaint_after_secs(0.05); }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (&app.browser.query, &self.view, &self.root, &app.ui.sample_favorites, &app.ui.recent_samples).hash(&mut hash);
            format!("{:016x}", hash.finish())
        };
        if key != self.filter_key {
            let words: Vec<_> = app.browser.query.split_whitespace().map(str::to_lowercase).collect();
            self.shown = self.index.entries.iter().enumerate().filter(|(_, entry)| words.iter().all(|word| entry.search.contains(word))
                && (self.root.is_empty() || Path::new(&entry.path).starts_with(&self.root))
                && (self.view != "Favorites" || app.ui.sample_favorites.contains(&entry.path))
                && (self.view != "Recent" || app.ui.recent_samples.contains(&entry.path))).map(|(index, _)| index).collect();
            if self.view == "Recent" { self.shown.sort_by_key(|index| app.ui.recent_samples.iter().position(|path| path == &self.index.entries[*index].path)); }
            self.filter_key = key;
        }
        ui.small(format!("{} samples", self.shown.len()));
        if self.index.truncated { ui.weak("Library limit reached. Add more specific folders to narrow the index."); }
        for error in self.index.errors.iter().take(3) { ui.small(error); }
        if app.ui.sample_folders.is_empty() { ui.label("Add your sample folders once. They stay available across sessions."); }
        let shown = self.shown.clone();
        egui::ScrollArea::vertical().id_salt("sample_library_results").show_rows(ui, 25.0, shown.len(), |ui, rows| {
            for row in rows {
                let entry = self.index.entries[shown[row]].clone();
                ui.horizontal(|ui| {
                    let favorite = app.ui.sample_favorites.contains(&entry.path);
                    if ui.small_button(if favorite { "★" } else { "☆" }).clicked() {
                        if favorite { app.ui.sample_favorites.retain(|path| path != &entry.path); }
                        else if app.ui.sample_favorites.len() < 4096 { app.ui.sample_favorites.push(entry.path.clone()); }
                        self.filter_key.clear();
                    }
                    let response = ui.add(egui::Button::selectable(self.selected == entry.path, &entry.name).sense(Sense::click_and_drag())).on_hover_text(format!("{}\n{:.1} MiB · double-click to load", entry.path, entry.bytes as f64 / 1_048_576.0));
                    response.dnd_set_drag_payload(BrowserItem::Sample(entry.path.clone()));
                    if response.clicked() { response.request_focus(); self.focus = Some(response.id); self.selected = entry.path.clone(); if app.ui.auto_preview_samples { self.preview(app, &entry.path); } }
                    if response.double_clicked() { self.import(app, &entry.path); }
                });
            }
        });
        if self.focus.is_some_and(|id| ui.memory(|memory| memory.has_focus(id))) {
            let movement = ui.input_mut(|input| if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { 1 } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { -1 } else { 0 });
            if movement != 0 && !shown.is_empty() {
                let current = shown.iter().position(|index| self.index.entries[*index].path == self.selected).unwrap_or(0);
                let next = current.saturating_add_signed(movement).min(shown.len() - 1);
                self.selected = self.index.entries[shown[next]].path.clone();
                if app.ui.auto_preview_samples { self.preview(app, &self.selected.clone()); }
            }
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) && !self.selected.is_empty() { self.import(app, &self.selected.clone()); }
        }
    }
}

pub fn poll(app: &mut SoundApp, ctx: &egui::Context) {
    let mut samples = std::mem::take(&mut app.browser.samples);
    samples.poll(app);
    if samples.preview_waiting || samples.scan.is_some() { ctx.request_repaint_after_secs(0.03); }
    app.browser.samples = samples;
}

pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let mut samples = std::mem::take(&mut app.browser.samples);
    samples.show(app, ui);
    app.browser.samples = samples;
}

pub fn search(app: &mut SoundApp, query: &str, limit: usize) -> serde_json::Value {
    let mut samples = std::mem::take(&mut app.browser.samples);
    samples.initialize(&app.ui.sample_folders);
    samples.poll(app);
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let found: Vec<_> = samples.index.entries.iter().filter(|entry| words.iter().all(|word| entry.search.contains(word))).take(limit.min(200)).map(|entry| serde_json::json!({"name": entry.name, "path": entry.path, "bytes": entry.bytes, "favorite": app.ui.sample_favorites.contains(&entry.path)})).collect();
    let result = serde_json::json!({"indexing": samples.scan.is_some(), "results": found, "truncated": samples.index.truncated});
    app.browser.samples = samples;
    result
}

pub fn preview(app: &mut SoundApp, path: &str) -> Result<(), String> {
    if app.player.as_ref().is_none_or(|player| player.silent) { return Err("Choose an audio output to preview samples".into()); }
    let mut samples = std::mem::take(&mut app.browser.samples);
    samples.preview(app, path);
    let queued = samples.preview_waiting;
    app.browser.samples = samples;
    if queued { Ok(()) } else { Err(app.ui.status.clone()) }
}

pub fn stop(app: &mut SoundApp) {
    if let Some(player) = &app.player { player.stop_sample_preview(); }
    if let Some(worker) = &mut app.browser.samples.preview { worker.generation = worker.generation.wrapping_add(1); }
    app.browser.samples.preview_waiting = false;
}
