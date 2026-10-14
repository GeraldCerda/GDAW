//! The MIDI Editor: a piano roll with a velocity lane, docked under the Edit window.

use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_model::{ClipContent, ClipId, Tool};
use soundcraft_time::{NoteValue, TICKS_PER_QUARTER};

const KEY_W: f32 = 48.0;
const ROW_H: f32 = 9.0;
const VEL_H: f32 = 64.0;

#[derive(Debug, Clone, Default)]
pub struct MidiEditorState {
    pub selected: Vec<usize>,
    pub drag: Option<NoteDrag>,
    pub scroll: f32,
    pub clip: Option<ClipId>,
    /// Rubber-band selection start (screen position).
    pub band: Option<egui::Pos2>,
}

#[derive(Debug, Clone)]
pub struct NoteDrag {
    pub index: usize,
    pub resize: bool,
    pub start: egui::Pos2,
    pub dx_ticks: i64,
    pub dy_semi: i64,
}

/// The MIDI clip being edited: the first selected MIDI clip, else a MIDI clip under the insertion
/// point on a selected track.
pub fn target_clip(app: &SoundApp) -> Option<ClipId> {
    let s = app.engine.session();
    for id in &s.edit.selected_clips {
        if let Some((_, c)) = s.find_clip(*id)
            && matches!(c.content, ClipContent::Midi { .. })
        {
            return Some(*id);
        }
    }
    let at = s.edit.selection.start;
    s.tracks
        .iter()
        .filter(|t| s.edit.selected_tracks.contains(&t.id) && t.kind.is_midi())
        .flat_map(|t| t.clips().iter())
        .find(|c| c.range().contains(at) || c.range().overlaps(&s.edit.selection))
        .or_else(|| s.tracks.iter().filter(|t| s.edit.selected_tracks.contains(&t.id) && t.kind.is_midi()).flat_map(|t| t.clips().iter()).next())
        .map(|c| c.id)
}

fn is_black(p: i32) -> bool {
    matches!(p.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

fn grid_stride(grid_ticks: i64, len_ticks: i64, width: f32) -> i64 {
    let grid = grid_ticks.max(1);
    let budget = (f64::from(width.max(1.0)) / 4.0).floor().clamp(1.0, 4096.0) as i64;
    let cells = len_ticks.max(1) / grid;
    let multiple = cells / budget + i64::from(cells % budget != 0);
    grid.saturating_mul(multiple.max(1))
}

fn snapped_delta(pixels: f32, px_per_tick: f64, grid_ticks: i64) -> i64 {
    if !pixels.is_finite() || !px_per_tick.is_finite() || px_per_tick <= 0.0 {
        return 0;
    }
    let grid = grid_ticks.max(1);
    ((f64::from(pixels) / px_per_tick / grid as f64).round() as i64).saturating_mul(grid)
}

fn selected_indices(result: &serde_json::Value) -> Vec<usize> {
    result["indices"].as_array().map(|indices| {
        indices.iter().filter_map(serde_json::Value::as_u64).filter_map(|index| usize::try_from(index).ok()).collect()
    }).unwrap_or_default()
}

fn resized_length(length: i64, delta: i64, grid: i64) -> i64 {
    length.saturating_add(delta).max(grid.min(length).max(1))
}

pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    // Painter-only contents do not advance the layout. Claim the resized
    // space, including the empty state, so the dock keeps its chosen height.
    ui.take_available_space();
    let t = Tokens::current();
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, t.panel_bg);
    let header = Rect::from_min_size(full.min, vec2(full.width(), 22.0));
    ui.painter().rect_filled(header, 0.0, t.panel_bg2);
    ui.painter().text(pos2(header.min.x + 8.0, header.center().y), Align2::LEFT_CENTER, "MIDI EDITOR", bold(11.5), t.header_text);
    let Some(cid) = target_clip(app) else {
        let track = {
            let s = app.engine.session();
            s.tracks.iter().find(|t| t.kind.is_midi() && s.edit.selected_tracks.contains(&t.id)).map(|t| (t.id, t.name.clone()))
        };
        let msg = match &track {
            Some((_, name)) => format!("Click to add a MIDI clip to {name} at the insertion point."),
            None => "Select a MIDI clip (or a MIDI track) to edit its notes.".into(),
        };
        ui.painter().text(full.center(), Align2::CENTER_CENTER, msg, regular(13.0), t.text_dim);
        let body = Rect::from_min_max(pos2(full.min.x, header.max.y), full.max);
        if let Some((id, _)) = track
            && ui.interact(body, ui.id().with("new_clip"), Sense::click()).clicked()
        {
            let sel = app.engine.session().edit.selection;
            if let Ok(v) = app.run("midi.new_clip", json!({"track": id.0, "start": sel.start, "end": sel.end})) {
                let _ = app.run("edit.select", json!({"clips": [v["clip"]]}));
            }
        }
        return;
    };
    let Some((track, clip)) = app.engine.session().find_clip(cid).map(|(t, c)| (t, c.clone())) else { return };
    let ClipContent::Midi { sequence } = &clip.content else { return };
    let track_color = app.engine.session().track(track).map_or([120, 120, 200], |t| t.color);
    if app.midi.clip != Some(cid) {
        app.midi.selected.clear();
        app.midi.drag = None;
        app.midi.band = None;
    }
    app.midi.selected.retain(|index| *index < sequence.notes.len());
    ui.painter().text(
        pos2(header.min.x + 110.0, header.center().y),
        Align2::LEFT_CENTER,
        format!(
            "{} · {} notes · {} selected · Double-click: add · drag: move/select · arrows: move",
            clip.name,
            sequence.notes.len(),
            app.midi.selected.len()
        ),
        regular(11.0),
        t.text_dim,
    );
    let grid_ticks = match app.engine.session().edit.grid {
        soundcraft_time::GridValue::Note { value, dotted, triplet } => {
            let mut g = if value == NoteValue::Bar {
                let session = app.engine.session();
                let at = session.tempo.samples_to_ticks(clip.start, session.sample_rate);
                session.tempo.meter_at_tick(at).ticks_per_bar()
            } else {
                value.ticks()
            };
            if dotted {
                g = g * 3 / 2;
            }
            if triplet {
                g = g * 2 / 3;
            }
            g.max(1)
        }
        _ => TICKS_PER_QUARTER / 4,
    };
    // Header toolbar: whole-clip MIDI operations.
    let bar = Rect::from_min_max(pos2(header.max.x - 465.0, header.min.y + 1.0), pos2(header.max.x - 6.0, header.max.y - 1.0));
    let mut tb = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    tb.spacing_mut().item_spacing.x = 4.0;
    let mut op: Option<(&str, serde_json::Value)> = None;
    if tb.small_button("Legato").clicked() {
        op = Some(("event.change_duration", json!({"legato": 0})));
    }
    if tb.small_button("Vel −10").clicked() {
        op = Some(("event.change_velocity", json!({"add": -10})));
    }
    if tb.small_button("Vel +10").clicked() {
        op = Some(("event.change_velocity", json!({"add": 10})));
    }
    if tb.small_button("Quantize").on_hover_text("Quantize to the grid").clicked() {
        op = Some(("event.quantize", json!({"grid": grid_ticks, "strength": 100})));
    }
    let selected_grid = app.engine.session().edit.grid;
    egui::ComboBox::from_id_salt(("midi-grid", cid.0)).width(100.0).selected_text(selected_grid.label()).show_ui(&mut tb, |ui| {
        for (label, value, note, triplet) in [
            ("1 bar", "1 bar", NoteValue::Bar, false),
            ("1/4", "1/4", NoteValue::Quarter, false),
            ("1/8", "1/8", NoteValue::Eighth, false),
            ("1/16", "1/16", NoteValue::Sixteenth, false),
            ("1/32", "1/32", NoteValue::ThirtySecond, false),
            ("1/64", "1/64", NoteValue::SixtyFourth, false),
            ("1/8T", "1/8t", NoteValue::Eighth, true),
            ("1/16T", "1/16t", NoteValue::Sixteenth, true),
            ("1/32T", "1/32t", NoteValue::ThirtySecond, true),
        ] {
            let chosen = soundcraft_time::GridValue::Note { value: note, dotted: false, triplet };
            if ui.selectable_label(selected_grid == chosen, label).clicked() {
                let _ = app.run("view.grid", json!({"value": value}));
            }
        }
    });
    if let Some((id, mut p)) = op {
        p["clips"] = json!([cid.0]);
        let _ = app.run(id, p);
        app.midi.selected.clear();
    }
    let s = app.engine.session_arc();
    let sr = s.sample_rate;
    let base_tick = s.tempo.samples_to_ticks(clip.start, sr);
    let len_ticks = s.tempo.samples_to_ticks(clip.end(), sr).saturating_sub(base_tick).max(1);
    let roll = Rect::from_min_max(pos2(full.min.x + KEY_W, header.max.y), pos2(full.max.x - 4.0, full.max.y - VEL_H - 4.0));
    let keys = Rect::from_min_max(pos2(full.min.x, roll.min.y), pos2(roll.min.x, roll.max.y));
    let vel = Rect::from_min_max(pos2(roll.min.x, roll.max.y + 4.0), pos2(roll.max.x, full.max.y - 2.0));
    // Vertical range: centre on the notes the first time.
    let rows = (roll.height() / ROW_H).floor() as i32;
    if app.midi.clip != Some(cid) {
        let hi = sequence.notes.iter().map(|n| i32::from(n.pitch)).max().unwrap_or(72);
        let lo = sequence.notes.iter().map(|n| i32::from(n.pitch)).min().unwrap_or(48);
        app.midi.scroll = (127 - (hi + lo) / 2 - rows / 2) as f32 * ROW_H;
        app.midi.clip = Some(cid);
    }
    if ui.rect_contains_pointer(roll) {
        app.midi.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    app.midi.scroll = app.midi.scroll.clamp(0.0, (128.0 * ROW_H - roll.height()).max(0.0));
    let scroll = app.midi.scroll;
    let px_per_tick = f64::from(roll.width().max(1.0)) / len_ticks as f64;
    let x_of = |tick: i64| (f64::from(roll.min.x) + tick as f64 * px_per_tick) as f32;
    let y_of = |pitch: i32| roll.min.y + (127 - pitch) as f32 * ROW_H - scroll;
    let pitch_at = |y: f32| 127 - ((y - roll.min.y + scroll) / ROW_H).floor() as i32;
    let painter = ui.painter().with_clip_rect(roll.union(keys));
    // Background rows and keys.
    let top = pitch_at(roll.min.y);
    for r in 0..=rows + 1 {
        let pitch = top - r;
        let y = y_of(pitch);
        let row = Rect::from_min_size(pos2(roll.min.x, y), vec2(roll.width(), ROW_H));
        painter.rect_filled(row, 0.0, if is_black(pitch) { Color32::from_rgb(30, 30, 32) } else { Color32::from_rgb(40, 40, 42) });
        let key = Rect::from_min_size(pos2(keys.min.x, y), vec2(KEY_W - 2.0, ROW_H - 1.0));
        painter.rect_filled(key, 1.0, if is_black(pitch) { Color32::from_rgb(20, 20, 20) } else { Color32::from_rgb(225, 225, 225) });
        if pitch.rem_euclid(12) == 0 && (0..=127).contains(&pitch) {
            painter.text(
                pos2(key.max.x - 3.0, key.center().y),
                Align2::RIGHT_CENTER,
                soundcraft_midi::note_name(pitch as u8),
                regular(8.0),
                Color32::from_rgb(40, 40, 40),
            );
            painter.line_segment([pos2(roll.min.x, y + ROW_H), pos2(roll.max.x, y + ROW_H)], Stroke::new(1.0, Color32::from_rgb(64, 64, 68)));
        }
    }
    // Grid.
    let stride = grid_stride(grid_ticks, len_ticks, roll.width());
    let mut tick = 0i64;
    for _ in 0..=4096 {
        if tick > len_ticks {
            break;
        }
        let x = x_of(tick);
        painter.line_segment([pos2(x, roll.min.y), pos2(x, roll.max.y)], Stroke::new(1.0, Color32::from_rgb(50, 50, 54)));
        let next = tick.saturating_add(stride);
        if next <= tick {
            break;
        }
        tick = next;
    }
    let first_bar = s.tempo.bar_beat_at_tick(base_tick).bar;
    let last_bar = s.tempo.bar_beat_at_tick(base_tick.saturating_add(len_ticks)).bar;
    let bar_stride = grid_stride(1, last_bar.saturating_sub(first_bar).max(1), roll.width());
    let mut bar = first_bar;
    for _ in 0..=4096 {
        if bar > last_bar {
            break;
        }
        let tick = s.tempo.bar_start_tick(bar).saturating_sub(base_tick);
        if (0..=len_ticks).contains(&tick) {
            let x = x_of(tick);
            painter.line_segment([pos2(x, roll.min.y), pos2(x, roll.max.y)], Stroke::new(1.0, Color32::from_rgb(80, 80, 86)));
        }
        let next = bar.saturating_add(bar_stride);
        if next <= bar {
            break;
        }
        bar = next;
    }
    // Notes.
    let base = egui::Color32::from_rgb(track_color[0], track_color[1], track_color[2]);
    let drag = app.midi.drag.clone();
    for (i, n) in sequence.notes.iter().enumerate() {
        let (mut st, mut ln, mut p) = (n.start, n.length, i32::from(n.pitch));
        if let Some(d) = &drag
            && (d.index == i || (!d.resize && app.midi.selected.contains(&i) && app.midi.selected.contains(&d.index)))
        {
            if d.resize {
                if d.index == i {
                    ln = resized_length(ln, d.dx_ticks, grid_ticks);
                }
            } else {
                st = st.saturating_add(d.dx_ticks).max(0);
                p = (p + d.dy_semi as i32).clamp(0, 127);
            }
        }
        let r = Rect::from_min_max(pos2(x_of(st), y_of(p) + 1.0), pos2(x_of(st.saturating_add(ln)).max(x_of(st) + 3.0), y_of(p) + ROW_H - 1.0));
        let sel = app.midi.selected.contains(&i);
        let k = 0.45 + 0.55 * f32::from(n.velocity) / 127.0;
        let fill = if sel {
            Color32::WHITE
        } else {
            Color32::from_rgb((f32::from(base.r()) * k) as u8, (f32::from(base.g()) * k) as u8, (f32::from(base.b()) * k) as u8)
        };
        painter.rect(r, 2.0, fill, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    }
    // Interaction.
    let resp = ui.interact(roll, ui.id().with(("roll", cid.0)), Sense::click_and_drag());
    let hit = |pos: egui::Pos2| -> Option<(usize, bool)> {
        sequence.notes.iter().enumerate().rev().find_map(|(i, n)| {
            let r = Rect::from_min_max(
                pos2(x_of(n.start), y_of(i32::from(n.pitch))),
                pos2(x_of(n.start.saturating_add(n.length)).max(x_of(n.start) + 3.0), y_of(i32::from(n.pitch)) + ROW_H),
            );
            r.contains(pos).then_some((i, pos.x > r.max.x - 5.0))
        })
    };
    let snap = |pixels: f32| snapped_delta(pixels, px_per_tick, grid_ticks);
    if resp.drag_started()
        && let Some(p) = ui.input(|input| input.pointer.press_origin()).filter(|point| roll.contains(*point))
    {
        if let Some((i, edge)) = hit(p) {
            if !app.midi.selected.contains(&i) {
                app.midi.selected = vec![i];
            }
            app.midi.drag = Some(NoteDrag { index: i, resize: edge, start: p, dx_ticks: 0, dy_semi: 0 });
        } else {
            app.midi.band = Some(p);
        }
    }
    // Rubber-band selection on empty space.
    if let (Some(a), Some(b)) = (app.midi.band, ui.ctx().pointer_latest_pos()) {
        let band = Rect::from_two_pos(a, b);
        ui.painter().rect(
            band,
            0.0,
            Color32::from_rgba_unmultiplied(120, 170, 255, 40),
            Stroke::new(1.0, Color32::from_rgb(120, 170, 255)),
            StrokeKind::Inside,
        );
        if resp.drag_stopped() {
            app.midi.selected = sequence
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    let r = Rect::from_min_max(
                        pos2(x_of(n.start), y_of(i32::from(n.pitch))),
                        pos2(x_of(n.start.saturating_add(n.length)), y_of(i32::from(n.pitch)) + ROW_H),
                    );
                    r.intersects(band)
                })
                .map(|(i, _)| i)
                .collect();
            app.midi.band = None;
        }
    }
    if resp.dragged()
        && let (Some(d), Some(p)) = (&mut app.midi.drag, resp.interact_pointer_pos())
    {
        d.dx_ticks = snap(p.x - d.start.x);
        d.dy_semi = -((p.y - d.start.y) / ROW_H).round() as i64;
    }
    if resp.drag_stopped()
        && let Some(d) = app.midi.drag.take()
    {
        if d.resize {
            if let Some(n) = sequence.notes.get(d.index)
                && let Ok(result) = app.run("midi.note_edit", json!({"clip": cid.0, "index": d.index, "length_ticks": resized_length(n.length, d.dx_ticks, grid_ticks)}))
            {
                app.midi.selected = result["index"].as_u64().map(|index| vec![index as usize]).unwrap_or_default();
            }
        } else if (d.dx_ticks != 0 || d.dy_semi != 0)
            && let Ok(result) = app.run("midi.notes_move", json!({"clip": cid.0, "indices": app.midi.selected, "ticks": d.dx_ticks, "semitones": d.dy_semi}))
        {
            app.midi.selected = selected_indices(&result);
        }
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match hit(p) {
            Some((i, _)) => {
                if ui.input(|x| x.modifiers.shift) {
                    if !app.midi.selected.contains(&i) {
                        app.midi.selected.push(i);
                    }
                } else {
                    app.midi.selected = vec![i];
                }
                // Audition on the instrument would go here (needs a live MIDI input path).
            }
            None if s.edit.tool == Tool::Pencil || resp.double_clicked() => {
                let pitch = pitch_at(p.y).clamp(0, 127);
                let ticks = (f64::from(p.x - roll.min.x) / px_per_tick).max(0.0);
                let start = ((ticks / grid_ticks as f64).floor() as i64).saturating_mul(grid_ticks);
                let _ = app.run(
                    "midi.note_add",
                    json!({"clip": cid.0, "pitch": pitch, "start_ticks": start.max(0), "length_ticks": grid_ticks, "velocity": 100}),
                );
                app.midi.selected.clear();
            }
            None => app.midi.selected.clear(),
        }
    }
    // Keyboard: Cmd+A select all; arrows transpose (Shift = octave) and move by the grid.
    if ui.rect_contains_pointer(roll) && !ui.ctx().egui_wants_keyboard_input() {
        let (all, up, down, left, right, shift) = ui.input(|i| {
            let arrows = !i.modifiers.alt && !i.modifiers.command && !i.modifiers.ctrl;
            (
                i.modifiers.command && i.key_pressed(egui::Key::A),
                arrows && i.key_pressed(egui::Key::ArrowUp),
                arrows && i.key_pressed(egui::Key::ArrowDown),
                arrows && !i.modifiers.shift && i.key_pressed(egui::Key::ArrowLeft),
                arrows && !i.modifiers.shift && i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.shift,
            )
        });
        if all {
            app.midi.selected = (0..sequence.notes.len()).collect();
        }
        let semi = if up {
            1
        } else if down {
            -1
        } else {
            0
        } * if shift { 12 } else { 1 };
        let ticks = if right {
            grid_ticks
        } else if left {
            -grid_ticks
        } else {
            0
        };
        if (semi != 0 || ticks != 0) && !app.midi.selected.is_empty()
            && let Ok(result) = app.run("midi.notes_move", json!({"clip": cid.0, "indices": app.midi.selected, "ticks": ticks, "semitones": semi}))
        {
            app.midi.selected = selected_indices(&result);
        }
    }
    if !app.midi.selected.is_empty()
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
    {
        let _ = app.run("midi.note_delete", json!({"clip": cid.0, "indices": app.midi.selected}));
        app.midi.selected.clear();
    }
    // Velocity lane.
    ui.painter().rect_filled(vel, 0.0, Color32::from_rgb(28, 28, 30));
    ui.painter().text(pos2(full.min.x + 6.0, vel.center().y), Align2::LEFT_CENTER, "Velocity", regular(10.0), t.text_dim);
    let vresp = ui.interact(vel, ui.id().with(("vel", cid.0)), Sense::click_and_drag());
    for (i, n) in sequence.notes.iter().enumerate() {
        let x = x_of(n.start);
        let h = (vel.height() - 4.0) * f32::from(n.velocity) / 127.0;
        let col = if app.midi.selected.contains(&i) { Color32::WHITE } else { base };
        ui.painter().line_segment([pos2(x + 1.0, vel.max.y - 2.0), pos2(x + 1.0, vel.max.y - 2.0 - h)], Stroke::new(3.0, col));
        ui.painter().circle_filled(pos2(x + 1.0, vel.max.y - 2.0 - h), 2.5, col);
    }
    if (vresp.clicked() || vresp.dragged())
        && let Some(p) = vresp.interact_pointer_pos()
        && let Some((i, n)) = sequence.notes.iter().enumerate().min_by_key(|(_, n)| ((x_of(n.start) - p.x).abs() * 10.0) as i64)
        && (x_of(n.start) - p.x).abs() < 8.0
    {
        let v = (((vel.max.y - 2.0 - p.y) / (vel.height() - 4.0)) * 127.0).round().clamp(1.0, 127.0) as i64;
        if let Ok(result) = app.engine.execute_merged("midi.note_edit", &json!({"clip": cid.0, "index": i, "velocity": v}), &format!("vel:{}", cid.0))
            && let Some(index) = result["index"].as_u64()
            && app.midi.selected.contains(&i)
        {
            app.midi.selected = vec![index as usize];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_grids_have_a_bounded_number_of_lines() {
        for length in [960, 960_000, i64::MAX] {
            let stride = grid_stride(60, length, 1200.0);
            assert!(stride >= 60);
            assert_eq!(stride % 60, 0);
            assert!(length / stride <= 300);
        }
        assert_eq!(grid_stride(240, 3840, 1200.0), 240);
    }

    #[test]
    fn pixel_deltas_snap_symmetrically_with_triplet_precision() {
        let scale = 0.125;
        assert_eq!(snapped_delta(40.0, scale, 320), 320);
        assert_eq!(snapped_delta(-40.0, scale, 320), -320);
        assert_eq!(snapped_delta(14.0, scale, 240), 0);
        assert_eq!(snapped_delta(16.0, scale, 240), 240);
        assert_eq!(snapped_delta(f32::NAN, scale, 240), 0);
        assert_eq!(snapped_delta(40.0, 0.0, 240), 0);
    }

    #[test]
    fn resizing_keeps_preview_and_commit_above_the_same_minimum() {
        assert_eq!(resized_length(480, -1000, 240), 240);
        assert_eq!(resized_length(60, -1000, 240), 60);
        assert_eq!(resized_length(i64::MAX, 240, 240), i64::MAX);
    }
}
