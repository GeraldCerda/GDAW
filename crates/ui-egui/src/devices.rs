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

pub struct BrowserState {
    pub query: String,
    pub category: String,
    pub grouping: PluginGrouping,
    pub format: PluginFormat,
    pub plugins: Vec<BrowserPlugin>,
    pub plugins_loaded: bool,
    pub plugin_scan: Option<std::sync::mpsc::Receiver<Vec<BrowserPlugin>>>,
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
            plugins_loaded: false,
            plugin_scan: None,
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
    app.browser.plugins_loaded = true;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        match std::thread::Builder::new().name("plugin browser scan".into()).spawn(move || {
            let _ = tx.send(crate::plugin_browser::catalog(rescan));
        }) {
            Ok(_) => app.browser.plugin_scan = Some(rx),
            Err(error) => app.ui.status = format!("Cannot scan plugins: {error}"),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        app.browser.plugins = crate::plugin_browser::catalog(rescan);
    }
}

fn poll_plugins(app: &mut SoundApp) {
    let result = app.browser.plugin_scan.as_ref().map(|scan| scan.try_recv());
    match result {
        Some(Ok(plugins)) => {
            app.browser.plugins = plugins;
            app.browser.plugin_scan = None;
        }
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
            app.browser.plugin_scan = None;
            app.ui.status = "Plugin scan stopped. Use Rescan installed plugins to retry.".into();
        }
        _ => {}
    }
}

pub(crate) fn prepare_plugins(app: &mut SoundApp) {
    poll_plugins(app);
    if !app.browser.plugins_loaded && app.browser.plugin_scan.is_none() && app.player.is_some() {
        scan_plugins(app, false);
    }
}

pub fn selected_track(app: &SoundApp) -> Option<TrackId> {
    let session = app.engine.session();
    session.edit.selected_tracks.iter().copied().find(|id| session.track(*id).is_some())
        .or_else(|| session.edit.selected_clips.iter().find_map(|id| session.find_clip(*id).map(|(track, _)| track)))
}

fn refresh_files(app: &mut SoundApp) {
    app.browser.files = std::fs::read_dir(&app.browser.folder).map(|entries| {
        entries.flatten().take(512).filter_map(|entry| {
            let path = entry.path();
            let directory = path.is_dir();
            let name = entry.file_name().to_string_lossy().into_owned();
            (directory || crate::sampler_window::is_audio_name(&name)).then_some((name, path, directory))
        }).collect()
    }).unwrap_or_default();
    app.browser.files.sort_by_cached_key(|(name, _, directory)| (!*directory, name.to_lowercase()));
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
    if info.is_instrument {
        return Err("Choose an instrument on an instrument track; insert slots accept effects".into());
    }
    let mut command = json!({"track": track.0, "slot": slot, "plugin": plugin});
    if let Some(params) = params {
        command["params"] = params;
    }
    app.run("mix.insert", command)?;
    let _ = app.run("edit.select", json!({"tracks": [track.0]}));
    app.ui.show_devices = true;
    app.ui.status = format!("{} loaded in insert {}", info.name, (b'A' + slot as u8) as char);
    Ok(())
}

fn row(app: &mut SoundApp, ui: &mut Ui, label: &str, item: BrowserItem) {
    let response = ui.add(egui::Label::new(label).sense(Sense::click_and_drag()));
    response.dnd_set_drag_payload(item.clone());
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
        ui.horizontal(|ui| {
            if ui.button("Browse folder").clicked()
                && let Some(path) = app.services.pick_folder.as_ref().and_then(|pick| pick("Sample folder"))
            {
                app.browser.folder = PathBuf::from(path);
                refresh_files(app);
            }
            if ui.small_button("Refresh").clicked() {
                refresh_files(app);
            }
            if ui.small_button("Up").clicked()
                && let Some(parent) = app.browser.folder.parent().map(PathBuf::from)
            {
                app.browser.folder = parent;
                refresh_files(app);
            }
        });
        ui.label(app.browser.folder.to_string_lossy().as_ref());
        let files = app.browser.files.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (name, path, directory) in files {
                if !name.to_lowercase().contains(&query) {
                    continue;
                }
                if directory {
                    if ui.button(format!("▸ {name}")).double_clicked() {
                        app.browser.folder = path;
                        refresh_files(app);
                    }
                } else {
                    row(app, ui, &name, BrowserItem::Sample(path.to_string_lossy().into_owned()));
                }
            }
        });
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
    egui::ComboBox::from_id_salt("browser_plugin_format").selected_text(app.browser.format.label()).show_ui(ui, |ui| {
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
    let rack_width = if insert.plugin == "drum_rack" { if insert.pad_samples.is_empty() { 420.0 } else { 620.0 } } else { 250.0 };
    ui.set_min_width(rack_width);
    ui.set_max_width(rack_width + 20.0);
    ui.horizontal(|ui| {
        ui.strong(if instrument { format!("Instrument · {name}") } else { format!("{} · {name}", (b'A' + slot as u8) as char) });
        if !instrument {
            if ui.small_button(if insert.bypass { "Enable" } else { "Bypass" }).clicked() {
                let _ = app.run("mix.insert_bypass", json!({"track": track.0, "slot": slot}));
            }
            if ui.small_button("×").clicked() {
                let _ = app.run("mix.insert_remove", json!({"track": track.0, "slot": slot}));
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
                            insert_item(app, track, &item);
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
        insert_item(app, track, &item);
    }
    if let Some(source) = response.dnd_release_payload::<crate::DragSource>() {
        let _ = app.run("mix.drum_pad_load", json!({"track": track.0, "note": note, "source": source.0}));
    }
}

pub fn chain(app: &mut SoundApp, ui: &mut Ui) {
    ui.ctx().request_repaint_after_secs(0.03);
    let Some(track) = selected_track(app).and_then(|id| app.engine.session().track(id).cloned()) else {
        ui.label("Select a track to view its devices");
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
            if let Some(instrument) = &track.instrument {
                egui::Frame::NONE.fill(Tokens::current().panel_bg).stroke(egui::Stroke::new(1.0, Tokens::current().border)).inner_margin(6.0).show(ui, |ui| {
                    ui.push_id(("device", track.id.0, soundcraft_mix::INSTRUMENT_SLOT), |ui| device(app, ui, track.id, soundcraft_mix::INSTRUMENT_SLOT, instrument));
                });
            }
            for (slot, insert) in track.mixer.inserts.iter().enumerate().filter_map(|(slot, insert)| insert.as_ref().map(|insert| (slot, insert))) {
                egui::Frame::NONE.fill(Tokens::current().panel_bg).stroke(egui::Stroke::new(1.0, Tokens::current().border)).inner_margin(6.0).show(ui, |ui| {
                    ui.push_id(("device", track.id.0, slot), |ui| device(app, ui, track.id, slot, insert));
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
                if let Some(slot) = track.mixer.inserts.iter().position(Option::is_none) {
                    ui.menu_button("Add effect to next empty insert", |ui| crate::mix_window::plugin_menu(app, ui, track.id, slot, false));
                } else {
                    ui.label("All effect slots are in use");
                }
                if track.kind == TrackKind::Instrument {
                    ui.menu_button("Choose instrument", |ui| crate::mix_window::instrument_menu(app, ui, track.id));
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
