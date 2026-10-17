use crate::plugin_browser::{BrowserPlugin, PluginFormat, PluginGrouping};
use crate::{SoundApp, theme::Tokens};
use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use soundcraft_model::{Insert, TrackId, TrackKind};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum BrowserItem {
    Plugin(String),
    Sample(String),
    Preset(String, String),
}

#[derive(Clone)]
pub struct InsertDrag { pub track: TrackId, pub slot: usize, pub plugin: String }

pub fn move_drop(app: &mut SoundApp, track: TrackId, slot: usize, source: &InsertDrag, copy: bool) {
    let _ = app.run("mix.insert_move", json!({"track": track.0, "from_track": source.track.0, "from": source.slot, "to": slot, "copy": copy, "expected_plugin": source.plugin}));
}

pub fn track_drop(app: &mut SoundApp, track: TrackId, response: &egui::Response) {
    if let Some(item) = response.dnd_release_payload::<BrowserItem>() { insert_item(app, track, &item); }
    if let Some(source) = response.dnd_release_payload::<InsertDrag>() {
        let slot = app.engine.session().track(track).and_then(|track| track.mixer.inserts.iter().enumerate().find(|(slot, insert)| insert.is_none() && track.instrument_insert_slot() != Some(*slot)).map(|(slot, _)| slot));
        if let Some(slot) = slot { move_drop(app, track, slot, &source, response.ctx.input(|input| input.modifiers.alt)); }
        else { app.ui.status = "This track has no free insert. Drop onto a specific slot to reorder or swap.".into(); }
    }
}

pub fn new_track_drop(app: &mut SoundApp, item: &BrowserItem) {
    let instrument = match item { BrowserItem::Plugin(id) | BrowserItem::Preset(id, _) => app.browser.catalog.iter().find(|plugin| &plugin.id == id).is_some_and(|plugin| plugin.is_instrument), BrowserItem::Sample(_) => false };
    let name = match item { BrowserItem::Plugin(id) | BrowserItem::Preset(id, _) => app.browser.catalog.iter().find(|plugin| &plugin.id == id).map(|plugin| plugin.name.clone()), BrowserItem::Sample(path) => std::path::Path::new(path).file_stem().map(|name| name.to_string_lossy().into_owned()) };
    if let Ok(result) = app.run("track.new", json!({"kind": if instrument { "instrument" } else { "audio" }, "format": "stereo", "count": 1, "name": name})) {
        if let Some(track) = result["tracks"][0].as_u64().map(TrackId) { let _ = app.run("edit.select", json!({"tracks": [track.0]})); insert_item(app, track, item); }
    }
}

pub struct PluginScanUpdate {
    format: PluginFormat,
    plugins: Vec<BrowserPlugin>,
}

pub struct BrowserState {
    pub query: String,
    pub category: String,
    pub grouping: PluginGrouping,
    pub format: PluginFormat,
    pub plugins: Vec<BrowserPlugin>,
    pub catalog: Vec<BrowserPlugin>,
    pub unavailable: std::collections::BTreeMap<String, String>,
    pub show_unavailable: bool,
    pub samples: crate::sample_browser::SampleBrowser,
    pub plugins_loaded: bool,
    pub plugin_scan: Option<std::sync::mpsc::Receiver<PluginScanUpdate>>,
    pub pending_formats: usize,
    pub startup_scan: bool,
    pub folder: PathBuf,
    pub library_loaded: String,
    pub library_files: Vec<PathBuf>,
    pub library_scan: Option<DrumLibraryScan>,
    pub auto_kits: std::collections::BTreeSet<TrackId>,
    pub files: Vec<(String, PathBuf, bool)>,
    pub pad: u8,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            query: String::new(),
            category: "Instruments".into(),
            grouping: PluginGrouping::Developer,
            format: PluginFormat::All,
            plugins: crate::plugin_browser::built_in_catalog(),
            catalog: crate::plugin_browser::built_in_catalog(),
            unavailable: std::collections::BTreeMap::new(),
            show_unavailable: false,
            samples: crate::sample_browser::SampleBrowser::default(),
            plugins_loaded: false,
            plugin_scan: None,
            pending_formats: 0,
            startup_scan: false,
            folder: PathBuf::new(),
            library_loaded: String::new(),
            library_files: Vec::new(),
            library_scan: None,
            auto_kits: std::collections::BTreeSet::new(),
            files: Vec::new(),
            pad: 36,
        }
    }
}

fn scan_plugins(app: &mut SoundApp, rescan: bool) {
    crate::plugin_scan::begin(rescan);
    app.browser.pending_formats = 0;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        for format in [PluginFormat::Au, PluginFormat::Vst3, PluginFormat::Clap] {
            let sender = tx.clone();
            match std::thread::Builder::new().name(format!("{} plugin scan", format.label())).spawn(move || {
                let plugins = crate::plugin_browser::catalog_format(format, true);
                let _ = sender.send(PluginScanUpdate { format, plugins });
            }) {
                Ok(_) => app.browser.pending_formats += 1,
                Err(error) => app.ui.status = format!("Cannot scan {} plugins: {error}", format.label()),
            }
        }
        drop(tx);
        app.browser.plugins_loaded = app.browser.pending_formats > 0;
        app.browser.plugin_scan = (app.browser.pending_formats > 0).then_some(rx);
    }
    #[cfg(target_arch = "wasm32")]
    {
        app.browser.catalog = crate::plugin_browser::catalog(rescan);
        refresh_available(app);
        app.browser.plugins_loaded = true;
    }
}

fn poll_plugins(app: &mut SoundApp) {
    loop {
        let result = app.browser.plugin_scan.as_ref().map(|scan| scan.try_recv());
        match result {
            Some(Ok(update)) => {
                app.browser.catalog.retain(|plugin| plugin.format != update.format);
                app.browser.catalog.extend(update.plugins);
                crate::plugin_browser::normalize(&mut app.browser.catalog);
                refresh_available(app);
                app.browser.pending_formats = app.browser.pending_formats.saturating_sub(1);
                if app.browser.pending_formats == 0 {
                    app.browser.plugin_scan = None;
                    app.browser.startup_scan = false;
                    let failures = crate::plugin_scan::progress().failures.len();
                    app.ui.status = format!("Plugin scan complete: {} installed, {failures} failed", app.browser.plugins.iter().filter(|plugin| plugin.format != PluginFormat::BuiltIn).count());
                    crate::plugin_scan::save_catalog(&app.browser.catalog);
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                app.browser.plugin_scan = None;
                app.browser.startup_scan = false;
                app.browser.plugins_loaded = false;
                app.ui.status = "Plugin scan stopped. Use Rescan installed plugins to retry.".into();
                break;
            }
            _ => break,
        }
    }
}

pub fn begin_startup_scan(app: &mut SoundApp) {
    if let Some(path) = crate::plugin_scan::cache_dir().map(|directory| directory.join("unavailable.json")) {
        if let Ok(metadata) = std::fs::metadata(&path) {
            if metadata.len() <= 33_554_432 {
                if let Ok(bytes) = std::fs::read(path) { app.browser.unavailable = serde_json::from_slice(&bytes).unwrap_or_default(); }
            }
        }
    }
    if let Some((plugins, unchanged)) = crate::plugin_scan::restore_catalog() {
        app.browser.catalog = plugins;
        app.browser.plugins_loaded = true;
        refresh_available(app);
        if unchanged { app.ui.status = format!("{} plugins ready from cache", app.browser.plugins.len()); return; }
        app.browser.startup_scan = false;
        scan_plugins(app, false);
        app.ui.status = "Plugins ready. Checking changed installations in the background…".into();
        return;
    }
    app.browser.startup_scan = true;
    scan_plugins(app, false);
    if app.browser.plugin_scan.is_none() { app.browser.startup_scan = false; }
}

fn refresh_available(app: &mut SoundApp) {
    app.browser.plugins = app.browser.catalog.iter().filter(|plugin| !app.browser.unavailable.contains_key(&plugin.id)).cloned().collect();
}

fn save_unavailable(app: &SoundApp) {
    let Some(directory) = crate::plugin_scan::cache_dir() else { return };
    if let Ok(bytes) = serde_json::to_vec(&app.browser.unavailable) {
        if std::fs::create_dir_all(&directory).is_ok() {
            let staged = directory.join(format!("unavailable.{}.partial", std::process::id()));
            if std::fs::write(&staged, bytes).is_ok() { let _ = std::fs::rename(staged, directory.join("unavailable.json")); }
        }
    }
}

pub fn exclude_plugin(app: &mut SoundApp, id: &str, reason: &str) {
    if !soundcraft_mix::is_third_party(id) || id.len() > 4096 { return; }
    if app.browser.unavailable.len() >= 4096 && !app.browser.unavailable.contains_key(id) { return; }
    app.browser.unavailable.insert(id.to_string(), reason.chars().filter(|character| !character.is_control()).take(512).collect());
    refresh_available(app);
    save_unavailable(app);
}

pub fn retry_plugin(app: &mut SoundApp, id: &str) {
    soundcraft_vst3_host::retry_plugin(id);
    soundcraft_clap_host::retry_plugin(id);
    soundcraft_au_host::retry_plugin(id);
    app.browser.unavailable.remove(id);
    refresh_available(app);
    save_unavailable(app);
    if let Some(player) = &app.player { player.retry_plugin(id, app.engine.session_arc()); }
}

pub fn startup(app: &mut SoundApp, ui: &mut Ui) -> bool {
    if !app.browser.startup_scan { return false; }
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        ui.heading("G");
        ui.label("Discovering installed plugins");
        ui.spinner();
        let progress = crate::plugin_scan::progress();
        ui.label(format!("{} bundles checked · {} formats remaining", progress.completed, app.browser.pending_formats));
        for path in progress.current.values() { ui.label(path); }
        if ui.button("Continue while scanning").clicked() { app.browser.startup_scan = false; }
    });
    ui.ctx().request_repaint_after_secs(0.05);
    true
}

pub(crate) fn prepare_plugins(app: &mut SoundApp) {
    poll_plugins(app);
    let failures = app.player.as_ref().map_or_else(Vec::new, soundcraft_playback::Player::take_plugin_load_failures);
    for (plugin, reason) in failures { exclude_plugin(app, &plugin, &reason); app.ui.status = reason; }
    if !app.browser.plugins_loaded && app.browser.plugin_scan.is_none() && app.player.is_some() {
        scan_plugins(app, false);
    }
}

pub fn selected_track(app: &SoundApp) -> Option<TrackId> {
    let session = app.engine.session();
    session.edit.selected_tracks.iter().copied().find(|id| session.track(*id).is_some())
        .or_else(|| session.edit.selected_clips.iter().find_map(|id| session.find_clip(*id).map(|(track, _)| track)))
}


pub fn insert_item(app: &mut SoundApp, track: TrackId, item: &BrowserItem) {
    let result = match item {
        BrowserItem::Plugin(plugin) => {
            if crate::mix_window::plugin_info(plugin).is_some_and(|info| info.is_instrument) {
                app.run("mix.instrument", json!({"track": track.0, "plugin": plugin}))
            } else {
                app.run("mix.insert", json!({"track": track.0, "plugin": plugin}))
            }
        }
        BrowserItem::Sample(path) => {
            let instrument = app.engine.session().track(track).is_some_and(|track| track.kind == TrackKind::Instrument);
            let rack = app.engine.session().track(track).and_then(|track| track.instrument.as_ref()).is_some_and(|insert| insert.plugin == "drum_rack");
            if rack {
                app.run("mix.drum_pad_load", json!({"track": track.0, "note": app.browser.pad, "path": path}))
            } else if instrument {
                app.run("mix.sampler_load", json!({"track": track.0, "path": path}))
            } else {
                app.run("file.import_audio", json!({"track": track.0, "path": path, "at": app.engine.session().edit.selection.start}))
            }
        }
        BrowserItem::Preset(plugin, path) => {
            let loaded = std::fs::metadata(path).ok().filter(|metadata| metadata.len() <= 1_048_576)
                .and_then(|_| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if let Some(preset) = loaded.filter(|preset| preset["format"] == "soundcraft-plugin-preset" && preset["plugin"] == plugin.as_str()) {
                let instrument = crate::mix_window::plugin_info(plugin).is_some_and(|info| info.is_instrument);
                app.run(if instrument { "mix.instrument" } else { "mix.insert" }, json!({"track": track.0, "plugin": plugin, "params": preset["params"]}))
            } else {
                Err("The preset is unreadable or does not match its device".into())
            }
        }
    };
    app.ui.show_devices = true;
    if let Ok(value) = &result {
        if matches!(item, BrowserItem::Plugin(_) | BrowserItem::Preset(_, _)) { app.show_insert_result(value); }
    }
    if result.is_ok() {
        if let BrowserItem::Sample(path) = item { app.ui.recent_samples.retain(|recent| recent != path); app.ui.recent_samples.insert(0, path.clone()); app.ui.recent_samples.truncate(64); }
    }
    app.ui.status = match result {
        Ok(_) => "Device or sample loaded on the selected track".into(),
        Err(error) => error,
    };
}

pub fn insert_effect_at(app: &mut SoundApp, track: TrackId, slot: usize, item: &BrowserItem) -> Result<(), String> {
    if slot >= soundcraft_model::INSERT_SLOTS {
        return Err("Choose an insert slot from A to J".into());
    }
    let (plugin, params) = match item {
        BrowserItem::Plugin(plugin) => (plugin, None),
        BrowserItem::Preset(plugin, path) => {
            let preset = std::fs::metadata(path).ok().filter(|metadata| metadata.len() <= 1_048_576)
                .and_then(|_| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(|preset| preset["format"] == "soundcraft-plugin-preset" && preset["plugin"] == plugin.as_str())
                .ok_or("The preset is unreadable or does not match its device")?;
            (plugin, preset.get("params").cloned())
        }
        BrowserItem::Sample(_) => return Err("Drop audio on a track or Drum Rack pad; insert slots accept effects".into()),
    };
    let info = crate::mix_window::plugin_info(plugin).ok_or("This effect is unavailable")?;
    let mut command = json!({"track": track.0, "slot": slot, "plugin": plugin});
    if let Some(params) = params {
        command["params"] = params;
    }
    let result = app.run("mix.insert", command)?;
    app.show_insert_result(&result);
    let _ = app.run("edit.select", json!({"tracks": [track.0]}));
    app.ui.show_devices = true;
    app.ui.status = format!("{} loaded in insert {}", info.name, (b'A' + slot as u8) as char);
    Ok(())
}

fn row(app: &mut SoundApp, ui: &mut Ui, label: &str, item: BrowserItem) {
    let response = ui.add(egui::Label::new(label).sense(Sense::click_and_drag()));
    response.dnd_set_drag_payload(item.clone());
    if let BrowserItem::Plugin(id) = &item {
        response.context_menu(|ui| {
            if ui.button("Hide from plugin list").clicked() { exclude_plugin(app, id, "Hidden by you. Activate or repair the plugin, then choose Retry."); ui.close(); }
        });
    }
    if response.double_clicked() {
        if let Some(track) = selected_track(app) {
            insert_item(app, track, &item);
        } else {
            app.ui.status = "Select a track first".into();
        }
    }
}

pub fn browser(app: &mut SoundApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.strong("BROWSER");
        if ui.small_button("Hide").clicked() {
            app.ui.show_browser = false;
        }
    });
    ui.add(egui::TextEdit::singleline(&mut app.browser.query).hint_text("Search devices and samples").desired_width(f32::INFINITY));
    ui.horizontal_wrapped(|ui| {
        for category in ["Instruments", "Effects", "Plug-ins", "Samples", "Presets"] {
            if ui.selectable_label(app.browser.category == category, category).clicked() {
                app.browser.category = category.into();
            }
        }
    });
    let query = app.browser.query.to_lowercase();
    let category = app.browser.category.clone();
    ui.separator();
    if category == "Samples" {
        crate::sample_browser::show(app, ui);
        return;
    }
    if category == "Presets" {
        let devices: Vec<_> = selected_track(app).and_then(|id| app.engine.session().track(id)).map(|track| {
            track.instrument.iter().chain(track.mixer.inserts.iter().flatten()).map(|insert| insert.plugin.clone()).collect()
        }).unwrap_or_default();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for plugin in devices {
                let Some(folder) = app.preset_dir.as_ref().map(|root| root.join(crate::preset_folder_name(&plugin))) else { continue };
                if let Ok(entries) = std::fs::read_dir(folder) {
                    for entry in entries.flatten().take(256) {
                        let path = entry.path();
                        let name = path.file_stem().and_then(|name| name.to_str()).unwrap_or("Preset").to_string();
                        if path.extension().is_some_and(|extension| extension == "json") && name.to_lowercase().contains(&query) {
                            row(app, ui, &name, BrowserItem::Preset(plugin.clone(), path.to_string_lossy().into_owned()));
                        }
                    }
                }
            }
        });
        return;
    }
    prepare_plugins(app);
    ui.horizontal_wrapped(|ui| {
        ui.label("Group by");
        ui.selectable_value(&mut app.browser.grouping, PluginGrouping::Developer, "Developer");
        ui.selectable_value(&mut app.browser.grouping, PluginGrouping::Effect, "Effect");
    });
    egui::ComboBox::new("browser_plugin_format", "Format").selected_text(app.browser.format.label()).show_ui(ui, |ui| {
        for format in [PluginFormat::All, PluginFormat::Vst3, PluginFormat::Au, PluginFormat::Clap, PluginFormat::BuiltIn] {
            ui.selectable_value(&mut app.browser.format, format, format.label());
        }
    });
    if ui.add_enabled(app.browser.plugin_scan.is_none(), egui::Button::new("Rescan installed plugins")).clicked() {
        scan_plugins(app, true);
    }
    if app.browser.plugin_scan.is_some() {
        ui.weak("Scanning installed plugins…");
        ui.ctx().request_repaint_after_secs(0.05);
    }
    ui.weak("Supported: VST3, Audio Units and CLAP. AAX and VST2 are not hosted.");
    if ui.selectable_label(app.browser.show_unavailable, format!("Unavailable / hidden ({})", app.browser.unavailable.len().saturating_add(crate::plugin_scan::progress().failures.len()))).clicked() { app.browser.show_unavailable = !app.browser.show_unavailable; }
    if app.browser.show_unavailable {
        let unavailable = app.browser.unavailable.clone();
        for (id, reason) in unavailable {
            let name = app.browser.catalog.iter().find(|plugin| plugin.id == id).map_or(id.as_str(), |plugin| plugin.name.as_str()).to_string();
            ui.horizontal(|ui| { ui.label(&name); if ui.small_button("Retry / re-enable").clicked() { retry_plugin(app, &id); } });
            ui.small(reason);
        }
        let failures = crate::plugin_scan::progress().failures;
        for (path, reason) in &failures { ui.label(std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy()); ui.small(reason); }
        if !failures.is_empty() && ui.add_enabled(app.browser.plugin_scan.is_none(), egui::Button::new("Retry failed scans")).clicked() { crate::plugin_scan::retry_failed_scans(); scan_plugins(app, false); }
        if app.browser.unavailable.is_empty() && failures.is_empty() { ui.weak("All known plugins are available."); }
        return;
    }
    let progress = crate::plugin_scan::progress();
    for path in progress.current.values() { ui.small(format!("Scanning {path}")); }
    if !progress.failures.is_empty() {
        egui::CollapsingHeader::new(format!("{} plugins could not be scanned", progress.failures.len())).show(ui, |ui| {
            for (path, error) in &progress.failures { ui.label(format!("{path}: {error}")); }
        });
    }
    let instruments = match category.as_str() {
        "Instruments" => Some(true),
        "Effects" => Some(false),
        _ => None,
    };
    let groups = crate::plugin_browser::groups(&app.browser.plugins, app.browser.grouping, app.browser.format, instruments, &query);
    if groups.is_empty() {
        ui.weak("No matching plugins");
    }
    egui::ScrollArea::vertical().id_salt("browser_plugin_groups").show(ui, |ui| {
        for (group, plugins) in groups {
            let id = ("browser_plugin_group", app.browser.grouping, app.browser.format, category.as_str(), group.as_str());
            egui::CollapsingHeader::new(format!("{group} ({})", plugins.len())).id_salt(id).default_open(!query.is_empty()).show(ui, |ui| {
                for plugin in plugins {
                    let label = format!("{} · {}", plugin.name, plugin.format.label());
                    row(app, ui, &label, BrowserItem::Plugin(plugin.id));
                }
            });
        }
    });
    ui.weak("Double-click to load, or drag into Track Devices");
}

fn device(app: &mut SoundApp, ui: &mut Ui, track: TrackId, slot: usize, insert: &Insert) {
    let instrument = slot == soundcraft_mix::INSTRUMENT_SLOT;
    let name = crate::mix_window::plugin_info(&insert.plugin).map_or(insert.plugin.as_str(), |info| info.name).to_string();
    let rack_width = if insert.plugin == "drum_rack" { if insert.pad_samples.is_empty() { 720.0 } else { 920.0 } } else { 250.0 };
    ui.set_min_width(rack_width);
    ui.set_max_width(rack_width + 20.0);
    ui.horizontal(|ui| {
        let visual = if instrument { app.engine.session().track(track).and_then(|track| track.instrument_insert_slot()).unwrap_or(0) } else { slot };
        let title = ui.add(egui::Label::new(egui::RichText::new(format!("{} · {name}", (b'A' + visual.min(25) as u8) as char)).strong()).sense(Sense::click_and_drag()));
        title.dnd_set_drag_payload(InsertDrag { track, slot: visual, plugin: insert.plugin.clone() });
        if let Some(source) = title.dnd_release_payload::<InsertDrag>() { move_drop(app, track, visual, &source, ui.input(|input| input.modifiers.alt)); }
        {
            if ui.small_button(if insert.bypass { "Enable" } else { "Bypass" }).clicked() {
                let _ = app.run("mix.insert_bypass", json!({"track": track.0, "slot": visual}));
            }
            if ui.small_button("×").clicked() {
                let _ = app.run("mix.insert_remove", json!({"track": track.0, "slot": visual}));
            }
        }
    });
    if soundcraft_mix::is_third_party(&insert.plugin) {
        if ui.button("Open plugin editor").clicked()
            && let Some(player) = &app.player
            && let Err(error) = player.open_editor(track, slot)
        {
            app.ui.status = error;
        }
    } else if insert.plugin == "drum_rack" {
        rack(app, ui, track, insert);
    } else {
        if instrument && insert.plugin == "sampler" {
            crate::sampler_window::sample_strip(app, ui, track, insert);
        }
        if let Some(info) = crate::mix_window::plugin_info(&insert.plugin) {
            egui::ScrollArea::vertical().max_height(180.0).id_salt(("device_parameters", track.0, slot)).show(ui, |ui| {
                for parameter in info.params {
                    let mut value = insert.params.get(parameter.id).copied().unwrap_or(parameter.default);
                    let response = ui.add(egui::Slider::new(&mut value, parameter.min..=parameter.max).text(parameter.name));
                    if response.changed() {
                        let command = if instrument { "mix.instrument_param" } else { "mix.insert_param" };
                        let _ = app.engine.execute_merged(command, &json!({"track": track.0, "slot": slot, "param": parameter.id, "value": value}), &format!("device:{}:{slot}:{}", track.0, parameter.id));
                        if let Some(player) = &app.player {
                            player.editor_set_param(track, slot, parameter.id, value);
                        }
                    }
                }
            });
        }
    }
}

fn drum_note_name(note: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", names.get(usize::from(note % 12)).copied().unwrap_or("?"), i16::from(note / 12) - 2)
}

fn rack(app: &mut SoundApp, ui: &mut Ui, track: TrackId, insert: &Insert) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            egui::Grid::new(("drum_pads", track.0)).spacing(vec2(3.0, 3.0)).show(ui, |ui| {
                for row in (0..4).rev() {
                    for column in 0..4 {
                        let note = (36 + row * 4 + column) as u8;
                        let name = insert.pad_samples.get(&note).and_then(|source| app.engine.session().source(*source)).map_or("Drop Sample", |source| source.name.as_str());
                        let short: String = name.chars().take(13).collect();
                        let response = ui.add_sized([64.0, 42.0], egui::Button::new(format!("{}\n{short}", drum_note_name(note))).selected(app.browser.pad == note)).on_hover_text(name);
                        app.pad_drop_targets.push((response.rect, track, note));
                        if response.clicked() {
                            app.browser.pad = note;
                            if insert.pad_samples.contains_key(&note) {
                                let _ = app.run("mix.drum_pad_audition", json!({"track": track.0, "note": note}));
                            }
                        }
                        if let Some(item) = response.dnd_release_payload::<BrowserItem>() {
                            app.browser.pad = note;
                            pad_item(app, track, note, None, &item);
                        }
                        if let Some(source) = response.dnd_release_payload::<crate::DragSource>() {
                            let _ = app.run("mix.drum_pad_load", json!({"track": track.0, "note": note, "source": source.0}));
                        }
                    }
                    ui.end_row();
                }
            });
            ui.weak("Pads C1–D#2 · MIDI 36–51");
        });
        ui.separator();
        ui.vertical(|ui| {
            let note = app.browser.pad;
            if insert.pad_samples.is_empty() {
                ui.set_min_width(140.0);
                ui.set_max_width(150.0);
                if ui.add_sized([140.0, 172.0], egui::Button::new("Drop an Instrument or Sample Here\n\nAssign drum sample library…")).clicked() {
                    assign_drum_library(app);
                }
                if ui.small_button("Load pad sample…").clicked()
                    && let Some(path) = app.services.pick_open.as_ref().and_then(|pick| pick("Drum pad sample", &["wav", "aif", "aiff", "flac", "mp3", "ogg", "m4a"]))
                {
                    let _ = app.run("mix.drum_pad_load", json!({"track": track.0, "note": note, "path": path}));
                }
                return;
            }
            ui.set_min_width(320.0);
            ui.set_max_width(340.0);
            let source = insert.pad_samples.get(&note).copied();
            let name = source.and_then(|id| app.engine.session().source(id)).map_or("Empty pad", |source| source.name.as_str()).to_string();
            ui.strong(format!("{} · {name}", drum_note_name(note)));
            pad_waveform(app, ui, track, note, source);
            ui.horizontal(|ui| {
                if ui.button("Load").clicked()
                    && let Some(path) = app.services.pick_open.as_ref().and_then(|pick| pick("Drum pad sample", &["wav", "aif", "aiff", "flac", "mp3", "ogg", "m4a"]))
                {
                    let _ = app.run("mix.drum_pad_load", json!({"track": track.0, "note": note, "path": path}));
                }
                if ui.button("Clear").clicked() {
                    let _ = app.run("mix.drum_pad_clear", json!({"track": track.0, "note": note}));
                }
                let mut mute = insert.params.get(&format!("pad{note}.mute")).copied().unwrap_or(0.0) >= 0.5;
                if ui.checkbox(&mut mute, "Mute").changed() {
                    let _ = app.run("mix.drum_pad_param", json!({"track": track.0, "note": note, "param": "mute", "value": if mute { 1.0 } else { 0.0 }}));
                }
            });
            ui.horizontal(|ui| {
                for (parameter, minimum, maximum) in [("gain", -48.0, 12.0), ("pan", -1.0, 1.0), ("tune", -48.0, 48.0)] {
                    let mut value = insert.params.get(&format!("pad{note}.{parameter}")).copied().unwrap_or(0.0);
                    if ui.add(egui::DragValue::new(&mut value).range(minimum..=maximum).prefix(format!("{parameter} "))).changed() {
                        let _ = app.engine.execute_merged("mix.drum_pad_param", &json!({"track": track.0, "note": note, "param": parameter, "value": value}), &format!("pad:{}:{note}:{parameter}", track.0));
                    }
                }
            });
        });
        ui.separator();
        let selected_pad = app.browser.pad;
        pad_effects(app, ui, track, insert, selected_pad);
    });
}

fn pad_waveform(app: &mut SoundApp, ui: &mut Ui, track: TrackId, note: u8, source: Option<soundcraft_model::SourceId>) {
    let (rect, response) = ui.allocate_exact_size(vec2(320.0, 90.0), Sense::click());
    app.pad_drop_targets.push((rect, track, note));
    let painter = ui.painter_at(rect);
    let tokens = Tokens::current();
    painter.rect_filled(rect, 2.0, tokens.playlist_bg);
    if let Some(audio) = source.and_then(|id| app.engine.session().pool.get(id)) {
        let frames = audio.frames().max(1);
        let columns = 320usize;
        let lanes = audio.peaks.len().clamp(1, 2);
        for (channel, peaks) in audio.peaks.iter().take(lanes).enumerate() {
            let samples = audio.channel(channel).unwrap_or_default();
            let height = rect.height() / lanes as f32;
            let middle = rect.min.y + height * (channel as f32 + 0.5);
            for column in 0..columns {
                let first = frames.saturating_mul(column) / columns;
                let last = (frames.saturating_mul(column + 1) / columns).max(first + 1);
                let (low, high) = peaks.minmax(samples, first, last);
                let x = rect.min.x + column as f32;
                painter.line_segment([egui::pos2(x, middle - high.clamp(-1.0, 1.0) * height * 0.45), egui::pos2(x, middle - low.clamp(-1.0, 1.0) * height * 0.45)], egui::Stroke::new(1.0, tokens.text));
            }
        }
    } else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Drop an Instrument or Sample Here\nClick to assign a drum sample library", crate::theme::regular(12.0), tokens.text_dim);
    }
    if response.clicked() && source.is_none() {
        assign_drum_library(app);
    }
    if let Some(item) = response.dnd_release_payload::<BrowserItem>() {
        app.browser.pad = note;
        pad_item(app, track, note, None, &item);
    }
    if let Some(source) = response.dnd_release_payload::<crate::DragSource>() {
        let _ = app.run("mix.drum_pad_load", json!({"track": track.0, "note": note, "source": source.0}));
    }
}

pub fn chain(app: &mut SoundApp, ui: &mut Ui) {
    if app.player.is_some() {
        ui.ctx().request_repaint_after_secs(0.03);
    }
    let Some(track) = selected_track(app).and_then(|id| app.engine.session().track(id).cloned()) else {
        let drop = ui.add_sized([ui.available_width(), 90.0], egui::Label::new("Drop a plugin or sample here to create a track").sense(Sense::hover()));
        if let Some(item) = drop.dnd_release_payload::<BrowserItem>() { new_track_drop(app, &item); }
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("DEVICES · {}", track.name));
        let input = ui.button(format!("Input: {}", crate::mix_window::input_name(app, &track)));
        crate::mix_window::route_popup(app, &input, track.id, true);
        let output = ui.button(format!("Output: {}", crate::mix_window::route_name(app, &track.mixer.output)));
        crate::mix_window::route_popup(app, &output, track.id, false);
        if ui.button("Mix").on_hover_text("Open Mixer · Cmd+=").clicked() {
            let _ = app.run("window.mix", json!({}));
        }
        if ui.small_button("Hide").clicked() {
            app.ui.show_devices = false;
        }
    });
    ui.horizontal_wrapped(|ui| {
        if track.kind == TrackKind::Audio {
            input_meter(app, ui, track.id);
            if let Some(problem) = input_route_problem(app, &track) {
                ui.colored_label(Tokens::current().text_dim, problem);
            }
            let mut monitor = track.mixer.input_monitor;
            if ui.checkbox(&mut monitor, "Monitor").changed() {
                let _ = app.run("mix.input_monitor", json!({"track": track.id.0, "value": monitor}));
            }
        }
        ui.label(format!("Input device: {}", app.input_status()));
        ui.label(format!("Output device: {}", app.output_status()));
        if ui.button("Audio Setup").clicked() {
            let _ = app.run("window.playback_engine", json!({}));
        }
    });
    ui.separator();
    let response = egui::ScrollArea::horizontal().id_salt("track_device_chain").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for visual in 0..=soundcraft_model::INSERT_SLOTS {
                let instrument = track.instrument_insert_slot() == Some(visual);
                let insert = if instrument { track.instrument.as_ref() } else { track.mixer.inserts.get(visual).and_then(Option::as_ref) };
                let Some(insert) = insert else { continue };
                let address = if instrument { soundcraft_mix::INSTRUMENT_SLOT } else { visual };
                egui::Frame::NONE.fill(Tokens::current().panel_bg).stroke(egui::Stroke::new(1.0, Tokens::current().border)).inner_margin(6.0).show(ui, |ui| {
                    ui.push_id(("device", track.id.0, address), |ui| device(app, ui, track.id, address, insert));
                });
            }
            ui.vertical(|ui| {
                ui.set_min_width(180.0);
                ui.label(if track.kind == TrackKind::Instrument { "Drop an instrument or effect" } else { "Drop an effect" });
                if track.instrument.as_ref().is_some_and(|insert| insert.plugin == "drum_rack") {
                    if ui.button(if app.ui.drum_library.is_empty() { "Assign drum sample library…" } else { "Drum sample library…" }).clicked() {
                        assign_drum_library(app);
                    }
                    if !app.ui.drum_library.is_empty() {
                        ui.small(app.ui.drum_library.as_str());
                        if app.browser.library_scan.is_some() {
                            ui.weak("Scanning drum library…");
                            ui.ctx().request_repaint_after_secs(0.05);
                        }
                        if ui.add_enabled(app.browser.library_scan.is_none() && !app.browser.library_files.is_empty(), egui::Button::new("Load random kit")).clicked() {
                            load_random_kit(app, track.id, true);
                        }
                        if ui.button("Rescan drum library").clicked() {
                            let path = app.ui.drum_library.clone();
                            let _ = app.run("ui.drum_library", json!({"path": path}));
                        }
                    }
                }
                if let Some(slot) = track.mixer.inserts.iter().enumerate().find(|(slot, insert)| insert.is_none() && track.instrument_insert_slot() != Some(*slot)).map(|(slot, _)| slot) {
                    ui.menu_button("Add effect to next empty insert", |ui| crate::mix_window::plugin_menu(app, ui, track.id, slot, false));
                } else {
                    ui.label("All effect slots are in use");
                }
                ui.allocate_space(vec2(180.0, 120.0));
            });
        });
    });
    let drop = ui.interact(response.inner_rect, ui.id().with("device_chain_drop"), Sense::hover());
    if let Some(item) = drop.dnd_release_payload::<BrowserItem>() {
        insert_item(app, track.id, &item);
    }
}

pub(crate) fn input_meter(app: &SoundApp, ui: &mut Ui, track: TrackId) {
    let peak = app.input_meters.get(&track).map_or(0.0, |meter| meter.level.iter().copied().fold(0.0f32, f32::max));
    let db = if peak > 0.0 && peak.is_finite() { 20.0 * peak.log10() } else { -60.0 };
    let text = if peak > 0.00001 { format!("Input {db:.1} dBFS") } else { "Input −∞ dBFS".into() };
    ui.add_sized([130.0, 16.0], egui::ProgressBar::new(((db + 60.0) / 60.0).clamp(0.0, 1.0)).text(text))
        .on_hover_text("Assigned physical input, before track processing. Metering does not enable software monitoring.");
}

pub(crate) fn input_route_problem(app: &SoundApp, track: &soundcraft_model::Track) -> Option<String> {
    let first = track.mixer.input.hardware_input_start()?;
    let channels = app.recorder.as_ref()?.channels;
    first.checked_add(track.channels()).is_none_or(|end| end > channels)
        .then(|| format!("{} is outside the opened {channels}-channel input device", crate::mix_window::input_name(app, track)))
}

pub struct DrumLibraryScan {
    path: String,
    receiver: std::sync::mpsc::Receiver<Result<Vec<PathBuf>, String>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for DrumLibraryScan {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn start_library_scan(path: String) -> Result<DrumLibraryScan, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let scan_path = path.clone();
    let scan_cancel = std::sync::Arc::clone(&cancel);
    let scan = move || {
        let result = drum_samples(std::path::Path::new(&scan_path), &scan_cancel);
        let _ = sender.send(result);
    };
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::thread::Builder::new().name("drum library scan".into()).spawn(scan).map_err(|error| error.to_string())?;
    #[cfg(target_arch = "wasm32")]
    scan();
    Ok(DrumLibraryScan { path, receiver, cancel })
}

fn drum_samples(root: &std::path::Path, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<PathBuf>, String> {
    let mut folders = vec![(root.to_path_buf(), 0usize)];
    let mut files = Vec::new();
    let mut inspected = 0usize;
    while let Some((folder, depth)) = folders.pop() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Drum library scan cancelled".into());
        }
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => return Err(format!("{}: {error}", folder.display())),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Drum library scan cancelled".into());
            }
            inspected += 1;
            if inspected > 8192 || files.len() >= 4096 {
                return Ok(files);
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() && depth < 8 {
                folders.push((path, depth + 1));
            } else if metadata.is_file() && metadata.len() > 0 && metadata.len() <= 16_777_216
                && crate::sampler_window::is_audio_name(&entry.file_name().to_string_lossy())
            {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn random_kit(files: &[PathBuf]) -> Vec<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos() as u64);
    let mut state = stamp ^ NEXT.fetch_add(1, Ordering::Relaxed);
    let mut files = files.to_vec();
    for index in (1..files.len()).rev() {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 31;
        files.swap(index, (value % (index as u64 + 1)) as usize);
    }
    files.into_iter().take(16).map(|path| path.to_string_lossy().into_owned()).collect()
}

pub fn prepare_drum_library(app: &mut SoundApp) {
    if app.ui.drum_library.is_empty() {
        app.browser.library_scan = None;
        app.browser.library_loaded.clear();
        app.browser.library_files.clear();
        app.browser.auto_kits.clear();
        return;
    }
    if app.browser.library_scan.as_ref().is_some_and(|scan| scan.path != app.ui.drum_library) {
        app.browser.library_scan = None;
    }
    if app.browser.library_loaded != app.ui.drum_library {
        app.browser.library_scan = None;
        app.browser.library_files.clear();
        app.browser.auto_kits.clear();
        app.browser.library_loaded = app.ui.drum_library.clone();
        match start_library_scan(app.ui.drum_library.clone()) {
            Ok(scan) => {
                app.browser.library_scan = Some(scan);
                app.ui.status = "Scanning drum sample library…".into();
            }
            Err(error) => {
                app.ui.status = format!("Cannot scan drum library: {error}");
                return;
            }
        }
    }
    let completed = app.browser.library_scan.as_ref().map(|scan| scan.receiver.try_recv());
    match completed {
        Some(Ok(result)) => {
            app.browser.library_scan = None;
            match result {
                Ok(files) => {
                    app.ui.status = if files.is_empty() { "Library assigned; no usable audio files found".into() } else { format!("Drum library assigned · {} audio files", files.len()) };
                    app.browser.library_files = files;
                }
                Err(error) => app.ui.status = format!("Cannot scan drum library: {error}"),
            }
        }
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
            app.browser.library_scan = None;
            app.ui.status = "Drum library scan stopped; rescan the library to retry".into();
        }
        _ => {}
    }
    if app.browser.library_scan.is_some() || app.browser.library_files.is_empty() {
        return;
    }
    let racks: Vec<_> = app.engine.session().tracks.iter().filter_map(|track| {
        track.instrument.as_ref().filter(|insert| insert.plugin == "drum_rack").map(|insert| (track.id, insert.pad_samples.is_empty()))
    }).collect();
    for (track, empty) in racks {
        if !app.browser.auto_kits.insert(track) || !empty {
            continue;
        }
        load_random_kit(app, track, false);
    }
}

fn load_random_kit(app: &mut SoundApp, track: TrackId, replace: bool) {
    let paths = random_kit(&app.browser.library_files);
    match app.run("mix.drum_kit_load", json!({"track": track.0, "paths": paths, "replace": replace, "skip_invalid": true})) {
        Ok(result) => {
            let loaded = result["loaded"].as_u64().unwrap_or(0);
            let skipped = result["skipped"].as_array().map_or(0, Vec::len);
            app.ui.status = if skipped == 0 { format!("Loaded {loaded} Drum Rack pads") } else { format!("Loaded {loaded} Drum Rack pads; skipped {skipped} unreadable samples") };
        }
        Err(error) => app.ui.status = format!("Drum library: {error}"),
    }
}

fn assign_drum_library(app: &mut SoundApp) {
    let start = if app.ui.drum_library.is_empty() { "Drum sample library" } else { app.ui.drum_library.as_str() };
    if let Some(path) = app.services.pick_folder.as_ref().and_then(|pick| pick(start)) {
        if let Err(error) = app.run("ui.drum_library", json!({"path": path})) {
            app.ui.status = error;
        } else {
            prepare_drum_library(app);
        }
    }
}

#[cfg(test)]
mod drum_library_tests {
    use super::*;

    #[test]
    fn random_kits_are_bounded_unique_and_never_invent_samples() {
        let files: Vec<_> = (0..40).map(|index| PathBuf::from(format!("sample-{index}.wav"))).collect();
        let kit = random_kit(&files);
        assert_eq!(kit.len(), 16);
        let unique: std::collections::BTreeSet<_> = kit.iter().collect();
        assert_eq!(unique.len(), 16);
        assert!(kit.iter().all(|path| files.contains(&PathBuf::from(path))));
        assert!(random_kit(&[]).is_empty());
        assert_eq!(random_kit(&files[..3]).len(), 3);
        assert_eq!(drum_note_name(36), "C1");
        assert_eq!(drum_note_name(51), "D#2");
    }
}

#[cfg(test)]
mod library_scan_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    #[test]
    fn discovery_runs_in_a_worker_and_filters_empty_files_and_symlinks() {
        let root = std::env::temp_dir().join(format!("gdaw-library-discovery-{}", std::process::id()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let sample = root.join("nested").join("kick.WAV");
        std::fs::write(&sample, b"sample").unwrap();
        std::fs::write(root.join("empty.wav"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"notes").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("nested"), root.join("linked")).unwrap();
        let scan = start_library_scan(root.to_string_lossy().into_owned()).unwrap();
        let result = scan.receiver.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(result, vec![sample]);
        drop(scan);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_a_library_cancels_pending_results_and_preserves_the_session() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, crate::Services::default());
        let before = app.engine.session().clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.browser.library_scan = Some(DrumLibraryScan { path: "old".into(), receiver, cancel: Arc::clone(&cancel) });
        app.browser.library_loaded = "old".into();
        app.browser.library_files.push("old.wav".into());
        sender.send(Ok(vec!["stale.wav".into()])).unwrap();
        prepare_drum_library(&mut app);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(app.browser.library_scan.is_none());
        assert!(app.browser.library_files.is_empty());
        assert!(app.browser.library_loaded.is_empty());
        assert_eq!(app.engine.session(), &before);
    }

    #[test]
    fn changing_a_library_discards_old_results_before_starting_its_scan() {
        let root = std::env::temp_dir().join(format!("gdaw-library-switch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, crate::Services::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.browser.library_scan = Some(DrumLibraryScan { path: "old".into(), receiver, cancel: Arc::clone(&cancel) });
        app.browser.library_loaded = "old".into();
        sender.send(Ok(vec!["stale.wav".into()])).unwrap();
        app.ui.drum_library = root.to_string_lossy().into_owned();
        prepare_drum_library(&mut app);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(app.browser.library_files.is_empty());
        assert_eq!(app.browser.library_loaded, app.ui.drum_library);
        if let Some(scan) = app.browser.library_scan.take() {
            assert!(scan.receiver.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap().is_empty());
        }
        std::fs::remove_dir(root).unwrap();
    }
}

fn pad_item(app: &mut SoundApp, track: TrackId, note: u8, slot: Option<usize>, item: &BrowserItem) {
    let result = match item {
        BrowserItem::Sample(path) => app.run("mix.drum_pad_load", json!({"track":track.0,"note":note,"path":path})),
        BrowserItem::Plugin(plugin) => {
            let mut params = json!({"track":track.0,"note":note,"action":"insert","plugin":plugin});
            if let Some(slot) = slot { params["slot"] = json!(slot); }
            app.run("mix.drum_pad_effect", params)
        }
        BrowserItem::Preset(plugin, path) => {
            let preset = std::fs::metadata(path).ok().filter(|metadata| metadata.len() <= 1_048_576)
                .and_then(|_| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if let Some(preset) = preset.filter(|preset| preset["format"] == "soundcraft-plugin-preset" && preset["plugin"] == plugin.as_str()) {
                let mut params = json!({"track":track.0,"note":note,"action":"insert","plugin":plugin,"params":preset["params"]});
                if let Some(slot) = slot { params["slot"] = json!(slot); }
                app.run("mix.drum_pad_effect", params)
            } else {
                Err("The preset is unreadable or does not match its device".into())
            }
        }
    };
    app.ui.status = match result { Ok(_) => format!("Loaded on pad {}", drum_note_name(note)), Err(error) => error };
}

fn pad_effects(app: &mut SoundApp, ui: &mut Ui, track: TrackId, instrument: &Insert, note: u8) {
    ui.vertical(|ui| {
        ui.set_min_width(270.0);
        ui.set_max_width(280.0);
        ui.strong(format!("{} · Effect chain", drum_note_name(note)));
        prepare_plugins(app);
        egui::ScrollArea::vertical().id_salt(("pad_effects",track.0,note)).max_height(190.0).show(ui, |ui| {
            let slots = instrument.pad_effects.get(&note);
            for slot in 0..soundcraft_model::INSERT_SLOTS {
                let insert = slots.and_then(|slots| slots.get(slot)).and_then(Option::as_ref);
                ui.push_id(("pad_effect",track.0,note,slot), |ui| {
                    let response = egui::Frame::NONE.fill(Tokens::current().panel_bg)
                        .stroke(egui::Stroke::new(1.0, Tokens::current().border)).inner_margin(3.0).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("{} ·", (b'A' + slot as u8) as char));
                            if let Some(insert) = insert {
                                let name = crate::mix_window::plugin_info(&insert.plugin).map_or(insert.plugin.as_str(), |info| info.name);
                                if ui.small_button(if insert.bypass { "Off" } else { "On" }).clicked() {
                                    let _ = app.run("mix.drum_pad_effect", json!({"track":track.0,"note":note,"slot":slot,"action":"bypass"}));
                                }
                                ui.label(name).on_hover_text(format!("{name} · pad {} · slot {}", drum_note_name(note), slot + 1));
                                if ui.small_button("×").on_hover_text("Remove this pad effect").clicked() {
                                    let _ = app.run("mix.drum_pad_effect", json!({"track":track.0,"note":note,"slot":slot,"action":"remove"}));
                                }
                            } else {
                                ui.menu_button("Add effect…", |ui| {
                                    let groups = crate::plugin_browser::groups(&app.browser.plugins, app.browser.grouping, app.browser.format, Some(false), &app.browser.query);
                                    for (group, effects) in groups {
                                        ui.menu_button(group, |ui| {
                                            for effect in effects {
                                                if soundcraft_dsp::is_generator(&effect.id) { continue; }
                                                if ui.button(format!("{} · {}", effect.name, effect.format.label())).clicked() {
                                                    pad_item(app, track, note, Some(slot), &BrowserItem::Plugin(effect.id.clone()));
                                                    ui.close();
                                                }
                                            }
                                        });
                                    }
                                });
                            }
                        });
                        if let Some(insert) = insert {
                            ui.horizontal(|ui| {
                                if soundcraft_mix::is_third_party(&insert.plugin) && ui.small_button("Open editor").clicked()
                                    && let (Some(player), Some(address)) = (&app.player, soundcraft_mix::drum_pad_slot(note,slot))
                                    && let Err(error) = player.open_editor(track,address)
                                { app.ui.status = error; }
                            });
                            if let Some(info) = crate::mix_window::plugin_info(&insert.plugin) {
                                egui::CollapsingHeader::new("Parameters").id_salt("parameters").show(ui, |ui| {
                                    for parameter in info.params {
                                        let mut value = insert.params.get(parameter.id).copied().unwrap_or(parameter.default);
                                        if ui.add(egui::Slider::new(&mut value,parameter.min..=parameter.max).logarithmic(parameter.taper == soundcraft_dsp::Taper::Log).text(parameter.name)).changed() {
                                            let _ = app.engine.execute_merged("mix.drum_pad_effect",
                                                &json!({"track":track.0,"note":note,"slot":slot,"action":"param","param":parameter.id,"value":value}),
                                                &format!("pad-effect:{}:{note}:{slot}:{}",track.0,parameter.id));
                                            if let (Some(player),Some(address)) = (&app.player,soundcraft_mix::drum_pad_slot(note,slot)) {
                                                player.editor_set_param(track,address,parameter.id,value);
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    });
                    let drop = ui.interact(response.response.rect,ui.id().with("drop"),Sense::hover());
                    if let Some(item) = drop.dnd_release_payload::<BrowserItem>() { pad_item(app,track,note,Some(slot),&item); }
                });
            }
        });
        ui.weak("Drop an effect into A–J · before track inserts");
    });
}
