use super::*;
use egui::{Align2, Rect, Stroke, StrokeKind, pos2};
use soundcraft_dsp::{ParamInfo, PluginInfo};

fn toggle(set: &mut std::collections::BTreeSet<(TrackId, usize)>, key: (TrackId, usize)) {
    if !set.remove(&key) { set.insert(key); }
}

fn parameter(app: &mut SoundApp, track: TrackId, slot: usize, parameter: &ParamInfo, value: f32) {
    let command = if slot == soundcraft_mix::INSTRUMENT_SLOT { "mix.instrument_param" } else { "mix.insert_param" };
    if app.engine.execute_merged(command, &json!({"track": track.0, "slot": slot, "param": parameter.id, "value": value}), &format!("device:{}:{slot}:{}", track.0, parameter.id)).is_ok() {
        if let Some(player) = &app.player { player.editor_set_param(track, slot, parameter.id, value); }
    }
}

fn axis(app: &mut SoundApp, ui: &mut Ui, track: TrackId, visual: usize, insert: &Insert, info: &PluginInfo, horizontal: bool) {
    let (name, current, other) = if horizontal { ("x", &insert.controls.x, &insert.controls.y) } else { ("y", &insert.controls.y, &insert.controls.x) };
    let mut selected = current.clone();
    let label = current.as_ref().and_then(|id| info.params.iter().find(|parameter| parameter.id == id)).map_or("none", |parameter| parameter.name);
    egui::ComboBox::from_id_salt(("axis", name)).width(98.0).selected_text(label).show_ui(ui, |ui| {
        ui.selectable_value(&mut selected, None, "none");
        for parameter in info.params.iter().filter(|parameter| other.as_deref() != Some(parameter.id)) {
            ui.selectable_value(&mut selected, Some(parameter.id.to_string()), parameter.name);
        }
    });
    if selected != *current {
        let mut params = json!({"track": track.0, "slot": visual});
        params[name] = json!(selected);
        let _ = app.run("mix.device_controls", params);
    }
}

fn xy(app: &mut SoundApp, ui: &mut Ui, track: TrackId, slot: usize, visual: usize, insert: &Insert, info: Option<&PluginInfo>, height: f32) {
    let tokens = Tokens::current();
    let (rect, response) = ui.allocate_exact_size(vec2(216.0, (height - 62.0).max(60.0)), Sense::click_and_drag());
    let area = rect.shrink(9.0);
    ui.painter().rect_filled(rect, 1.0, tokens.playlist_bg);
    let x = info.and_then(|info| insert.controls.x.as_ref().and_then(|id| info.params.iter().find(|parameter| parameter.id == id)));
    let y = info.and_then(|info| insert.controls.y.as_ref().and_then(|id| info.params.iter().find(|parameter| parameter.id == id)));
    let normalized = |parameter: &ParamInfo| parameter.to_normalized(insert.params.get(parameter.id).copied().unwrap_or(parameter.default));
    let mut location = pos2(area.min.x + x.map_or(0.0, normalized) * area.width(), area.max.y - y.map_or(0.0, normalized) * area.height());
    if (response.dragged() || response.clicked()) && let Some(pointer) = response.interact_pointer_pos() {
        location = pointer.clamp(area.min, area.max);
        if let Some(x) = x { parameter(app, track, slot, x, x.from_normalized(((location.x - area.min.x) / area.width()).clamp(0.0, 1.0))); }
        if let Some(y) = y { parameter(app, track, slot, y, y.from_normalized(((area.max.y - location.y) / area.height()).clamp(0.0, 1.0))); }
    }
    if x.is_some() || y.is_some() {
        ui.painter().line_segment([pos2(location.x, area.min.y), pos2(location.x, area.max.y)], Stroke::new(0.5, tokens.border_light));
        ui.painter().line_segment([pos2(area.min.x, location.y), pos2(area.max.x, location.y)], Stroke::new(0.5, tokens.border_light));
    }
    ui.painter().circle_stroke(location, 7.0, Stroke::new(1.7, tokens.accent));
    response.on_hover_text("Assign X and Y below, then drag to control both parameters. Values are saved with this device.");
    if let Some(info) = info {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            axis(app, ui, track, visual, insert, info, true);
            axis(app, ui, track, visual, insert, info, false);
        });
    } else { ui.weak("Unavailable · saved state retained"); }
}

fn controls(app: &mut SoundApp, ui: &mut Ui, track: TrackId, slot: usize, visual: usize, insert: &Insert, info: &PluginInfo, height: f32) {
    let key = (track, slot);
    ui.set_width(188.0);
    let configuring = app.browser.configuring.contains(&key);
    if ui.selectable_label(configuring, "Configure").on_hover_text("Choose controls here, or touch them in the native editor while Configure is enabled").clicked() {
        toggle(&mut app.browser.configuring, key);
    }
    egui::ScrollArea::vertical().id_salt(("published_parameters", track, slot)).max_height((height - 66.0).max(50.0)).show(ui, |ui| {
        if info.params.is_empty() { ui.weak("This plugin publishes no host parameters. Use its editor."); }
        for parameter_info in info.params {
            let mut pinned = insert.controls.parameters.iter().any(|id| id == parameter_info.id);
            if configuring {
                if ui.checkbox(&mut pinned, parameter_info.name).changed() {
                    let mut parameters = insert.controls.parameters.clone();
                    if pinned { parameters.push(parameter_info.id.to_string()); } else { parameters.retain(|id| id != parameter_info.id); }
                    let _ = app.run("mix.device_controls", json!({"track": track.0, "slot": visual, "parameters": parameters}));
                }
            } else if pinned {
                let original = insert.params.get(parameter_info.id).copied().unwrap_or(parameter_info.default);
                let mut value = parameter_info.to_normalized(original);
                ui.small(parameter_info.name);
                let response = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
                ui.small(parameter_info.format(parameter_info.from_normalized(value)));
                if response.changed() { parameter(app, track, slot, parameter_info, parameter_info.from_normalized(value)); }
            }
        }
        if !configuring && insert.controls.parameters.is_empty() { ui.weak("Choose Configure to expose controls."); }
    });
}

fn device(app: &mut SoundApp, ui: &mut Ui, track: TrackId, slot: usize, insert: &Insert, height: f32) {
    let visual = if slot == soundcraft_mix::INSTRUMENT_SLOT { app.engine.session().track(track).and_then(|track| track.instrument_insert_slot()).unwrap_or(0) } else { slot };
    let key = (track, slot);
    let info = crate::mix_window::plugin_info(&insert.plugin);
    let name = info.map_or(insert.plugin.as_str(), |info| info.name);
    let folded = app.browser.folded_devices.contains(&key);
    let expanded = app.browser.expanded_controls.contains(&key) && info.is_some();
    let rack = insert.plugin == "drum_rack";
    let sampler = insert.plugin == "sampler";
    let width = if folded { 40.0 } else if rack { if insert.pad_samples.is_empty() { 720.0 } else { 920.0 } } else if sampler { if expanded { 528.0 } else { 330.0 } } else if expanded { 418.0 } else { 226.0 };
    let tokens = Tokens::current();
    egui::Frame::NONE.fill(tokens.panel_bg2).stroke(Stroke::new(1.0, if app.browser.selected_device == Some(key) { tokens.accent } else { tokens.border_light })).inner_margin(5.0).show(ui, |ui| {
        ui.set_min_size(vec2(width, height));
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing = vec2(3.0, 4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (rect, active) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::click());
            ui.painter().circle_filled(rect.center(), 5.0, if insert.bypass || !insert.active { tokens.slot_mark } else { tokens.mode_on });
            if active.on_hover_text("Activate / bypass").clicked() { let _ = app.run("mix.insert_bypass", json!({"track": track.0, "slot": visual})); }
            if folded { if ui.small_button(">").clicked() { app.browser.folded_devices.remove(&key); } return; }
            if !rack && ui.small_button(if expanded { "<" } else { ">" }).on_hover_text("Show / hide configured parameter controls").clicked() { toggle(&mut app.browser.expanded_controls, key); }
            if ui.small_button("↗").on_hover_text("Open plugin editor").clicked() { app.show_insert_result(&json!({"track": track.0, "slot": visual})); }
            let title = ui.add_sized([if rack { width - 114.0 } else { 104.0 }, 18.0], egui::Label::new(egui::RichText::new(name).strong().size(11.0)).truncate().sense(Sense::click_and_drag()));
            title.dnd_set_drag_payload(InsertDrag { track, slot: visual, plugin: insert.plugin.clone() });
            chain_drop(app, ui, track, visual, &title);
            if title.clicked() { app.browser.selected_device = Some(key); }
            if title.double_clicked() { toggle(&mut app.browser.folded_devices, key); }
            title.context_menu(|ui| {
                if ui.button("Fold device").clicked() { app.browser.folded_devices.insert(key); ui.close(); }
                if ui.button("Remove device").clicked() { let _ = app.run("mix.insert_remove", json!({"track": track.0, "slot": visual})); ui.close(); }
            });
            if ui.small_button("↔").on_hover_text("Replace this device from the Browser").clicked() {
                app.browser.replace_device = Some(InsertDrag { track, slot: visual, plugin: insert.plugin.clone() });
                app.browser.category = if info.is_some_and(|info| info.is_instrument) { "Instruments" } else { "Effects" }.into();
                app.browser.query.clear();
                app.ui.show_browser = true;
            }
            ui.menu_button("▣", |ui| {
                if let Some(info) = info { crate::panels::preset_menu(app, ui, track, visual, info, insert); }
                else { ui.weak("This device is unavailable"); }
            }).response.on_hover_text("Load / save parameter presets");
        });
        if folded {
            ui.add(egui::Label::new(name).wrap());
        } else if rack {
            egui::ScrollArea::vertical().max_height((height - 28.0).max(60.0)).id_salt(("rack_body", track, slot)).show(ui, |ui| super::rack(app, ui, track, insert));
        } else if sampler {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(320.0);
                    egui::ScrollArea::vertical().max_height((height - 28.0).max(60.0)).id_salt(("sampler_body", track, slot)).show(ui, |ui| crate::sampler_window::sample_strip(app, ui, track, insert));
                });
                if expanded && let Some(info) = info { controls(app, ui, track, slot, visual, insert, info, height); }
            });
        } else {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| xy(app, ui, track, slot, visual, insert, info, height));
                if expanded && let Some(info) = info { controls(app, ui, track, slot, visual, insert, info, height); }
            });
        }
    });
}

pub(super) fn chain(app: &mut SoundApp, ui: &mut Ui) {
    if app.player.is_some() { ui.ctx().request_repaint_after_secs(0.03); }
    let tokens = Tokens::current();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ui.available_height().max(100.0)), Sense::hover());
    let width = app.browser.info_width.clamp(150.0, (rect.width() * 0.4).max(150.0));
    let info_rect = Rect::from_min_max(rect.min, pos2((rect.min.x + width).min(rect.max.x), rect.max.y));
    let divider = Rect::from_min_max(pos2(info_rect.max.x, rect.min.y), pos2((info_rect.max.x + 8.0).min(rect.max.x), rect.max.y));
    let response = ui.interact(divider, ui.id().with("device_info_divider"), Sense::drag());
    if response.dragged() { app.browser.info_width = (width + response.drag_delta().x).clamp(150.0, (rect.width() * 0.4).max(150.0)); }
    ui.painter().rect_filled(info_rect, 2.0, tokens.panel_bg);
    ui.painter().line_segment([divider.center_top(), divider.center_bottom()], Stroke::new(1.0, tokens.border_light));
    let track = selected_track(app).and_then(|id| app.engine.session().track(id).cloned());
    let mut info = ui.new_child(egui::UiBuilder::new().id_salt("device_info").max_rect(info_rect.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    info.set_clip_rect(info_rect);
    if let Some(track) = &track {
        info.strong(&track.name);
        info.add_space(8.0);
        info.small("Audio From");
        let input = info.button(crate::mix_window::input_name(app, track)).on_hover_text(app.input_status());
        crate::mix_window::route_popup(app, &input, track.id, true);
        info.small("Audio To");
        let output = info.button(crate::mix_window::route_name(app, &track.mixer.output)).on_hover_text(app.output_status());
        crate::mix_window::route_popup(app, &output, track.id, false);
        if track.kind == TrackKind::Audio {
            super::monitor(app, &mut info, track);
            input_meter(app, &mut info, track.id);
            if info.selectable_label(track.mixer.record_arm, "Arm recording").clicked() { let _ = app.run("mix.record_arm", json!({"track": track.id.0, "exclusive": !info.input(|input| input.modifiers.command)})); }
        }
        info.add_space(8.0);
        info.weak("Drag titles to move devices. Option-drag copies. Drop between devices to insert.");
    } else { info.strong("Device View"); info.weak("Select a track to see its devices and routing."); }
    let chain_rect = Rect::from_min_max(pos2(divider.max.x + 4.0, rect.min.y + 3.0), rect.max - vec2(3.0, 3.0));
    if chain_rect.width() <= 0.0 { return; }
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("device_chain").max_rect(chain_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(chain_rect);
    let height = (chain_rect.height() - 25.0).max(80.0);
    egui::ScrollArea::horizontal().id_salt("track_device_chain").auto_shrink([false, false]).show(&mut child, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let content_start = ui.cursor().min.x;
            let mut end = 0;
            if let Some(track) = &track {
                for visual in 0..=soundcraft_model::INSERT_SLOTS {
                    let instrument = track.instrument_insert_slot() == Some(visual);
                    let insert = if instrument { track.instrument.as_ref() } else { track.mixer.inserts.get(visual).and_then(Option::as_ref) };
                    let Some(insert) = insert else { continue };
                    chain_gap(app, ui, track.id, visual, height);
                    let slot = if instrument { soundcraft_mix::INSTRUMENT_SLOT } else { visual };
                    ui.push_id(("device", track.id, slot), |ui| device(app, ui, track.id, slot, insert, height));
                    end = visual.saturating_add(1).min(soundcraft_model::INSERT_SLOTS);
                }
                chain_gap(app, ui, track.id, end, height);
            }
            let width = (chain_rect.width() - (ui.cursor().min.x - content_start)).max(200.0);
            let (drop, response) = ui.allocate_exact_size(vec2(width, height + 10.0), Sense::hover());
            let hovering = response.dnd_hover_payload::<BrowserItem>().is_some() || response.dnd_hover_payload::<InsertDrag>().is_some();
            ui.painter().rect_filled(drop, 2.0, tokens.panel_bg);
            if hovering { ui.painter().rect_stroke(drop.shrink(1.0), 2.0, Stroke::new(1.5, tokens.accent), StrokeKind::Inside); }
            ui.painter().text(drop.center(), Align2::CENTER_CENTER, if track.is_some() { "Drop Audio Effects Here" } else { "Drop Files and Devices Here" }, crate::theme::regular(12.0), tokens.text_dim);
            if let Some(track) = &track {
                chain_drop(app, ui, track.id, end, &response);
                response.context_menu(|ui| {
                    if let Some(slot) = track.mixer.inserts.iter().enumerate().find(|(slot, insert)| insert.is_none() && track.instrument_insert_slot() != Some(*slot)).map(|(slot, _)| slot) { crate::mix_window::plugin_menu(app, ui, track.id, slot, false); }
                    else { ui.weak("All inserts are in use"); }
                });
            } else if let Some(item) = response.dnd_release_payload::<BrowserItem>() { new_track_drop(app, &item); }
        });
    });
}
