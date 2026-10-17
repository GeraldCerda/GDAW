use crate::{Engine, Result, EngineError};
use serde_json::{Value, json};
use soundcraft_model::{AutoParam, Insert, Session, Track, TrackId, TrackKind, INSERT_SLOTS};

#[derive(Clone)]
struct Device { insert: Insert, instrument: bool, track: TrackId, slot: usize, automated: bool }

fn error(message: impl Into<String>) -> EngineError { EngineError::BadParams("mix.insert_move".into(), message.into()) }

fn slots(track: &Track) -> Vec<Option<Device>> {
    let mut slots: Vec<Option<Device>> = (0..=INSERT_SLOTS).map(|_| None).collect();
    for (slot, insert) in track.mixer.inserts.iter().enumerate().take(INSERT_SLOTS) {
        slots[slot] = insert.as_ref().map(|insert| Device { insert: insert.clone(), instrument: false, track: track.id, slot, automated: true });
    }
    if let (Some(insert), Some(slot)) = (&track.instrument, track.instrument_insert_slot()) {
        slots[slot] = Some(Device { insert: insert.clone(), instrument: true, track: track.id, slot, automated: track.instrument_slot.is_some() });
    }
    slots
}

fn rebuild(original: &Session, track: &mut Track, slots: &[Option<Device>]) -> Result<()> {
    if slots.iter().flatten().filter(|device| device.instrument).count() > 1 { return Err(error("An instrument track can contain one instrument. Drop the device onto an empty instrument track instead.")); }
    if slots.iter().flatten().any(|device| device.instrument) && !matches!(track.kind, TrackKind::Instrument | TrackKind::Midi) { return Err(error("Drop instruments onto an instrument track or empty track space.")); }
    if slots.get(INSERT_SLOTS).and_then(Option::as_ref).is_some_and(|device| !device.instrument) { return Err(error("This legacy track has eleven devices. Free an insert before reordering the overflow instrument.")); }
    track.instrument = None;
    track.instrument_slot = None;
    track.mixer.inserts.iter_mut().for_each(|insert| *insert = None);
    track.automation.retain(|lane| !matches!(lane.param, AutoParam::Plugin { slot, .. } if usize::from(slot) <= INSERT_SLOTS));
    for (slot, device) in slots.iter().enumerate() {
        let Some(device) = device else { continue };
        if device.instrument { track.instrument = Some(device.insert.clone()); track.instrument_slot = Some(slot); }
        else if let Some(insert) = track.mixer.inserts.get_mut(slot) { *insert = Some(device.insert.clone()); }
        if device.automated {
            if let Some(source) = original.track(device.track) {
                for lane in source.automation.iter().filter(|lane| matches!(&lane.param, AutoParam::Plugin { slot, .. } if usize::from(*slot) == device.slot)) {
                    let mut lane = lane.clone();
                    if let AutoParam::Plugin { slot: index, .. } = &mut lane.param { *index = slot as u8; }
                    track.automation.push(lane);
                }
            }
        }
    }
    Ok(())
}

fn route_values(original: &Session, staged: &mut Session, track: TrackId, slots: &[Option<Device>]) {
    let sidechain = format!("plugin_sidechain.{}.", track.0);
    let output = format!("plugin_output.{}.", track.0);
    staged.edit.values.retain(|key, _| !key.starts_with(&sidechain) && !key.starts_with(&output));
    for (slot, device) in slots.iter().enumerate() {
        let Some(device) = device else { continue };
        if let Some(bus) = soundcraft_mix::plugin_sidechain(original, device.track, device.slot) { staged.edit.values.insert(format!("plugin_sidechain.{}.{slot}", track.0), bus.0 as f64); }
        for output in 1..16 {
            if let Some(bus) = soundcraft_mix::plugin_output(original, device.track, device.slot, output) { staged.edit.values.insert(format!("plugin_output.{}.{slot}.{output}", track.0), bus.0 as f64); }
        }
    }
}

fn reset_slots(track: TrackId, before: &[Option<Device>], after: &[Option<Device>], reset: &mut Vec<Value>) {
    for (slot, (before, after)) in before.iter().zip(after).enumerate() {
        let before_key = before.as_ref().map(|device| (device.track, device.slot));
        let after_key = after.as_ref().map(|device| (device.track, device.slot));
        if before_key == after_key { continue; }
        for device in [before, after].into_iter().flatten() {
            let value = json!({"track": track.0, "slot": slot, "instrument": device.instrument});
            if !reset.contains(&value) { reset.push(value); }
        }
    }
}

pub fn move_insert(engine: &mut Engine, params: &Value) -> Result<Value> {
    let target = crate::cmd::tracks_required(engine, "mix.insert_move", params)?.first().copied().ok_or_else(|| error("Choose a destination track"))?;
    let source = if let Some(track) = params.get("from_track") { crate::cmd::tracks_required(engine, "mix.insert_move", &json!({"track": track}))?.first().copied().ok_or_else(|| error("Choose a source track"))? } else { target };
    let parse = |name: &str| -> Result<usize> {
        let value = params.get(name).ok_or_else(|| error(format!("{name} is required")))?;
        let index = value.as_u64().and_then(|value| usize::try_from(value).ok()).or_else(|| value.as_str().and_then(|value| { let bytes = value.as_bytes(); if bytes.len() == 1 && bytes[0].is_ascii_alphabetic() { Some(usize::from(bytes[0].to_ascii_uppercase() - b'A')) } else { value.parse().ok() } }));
        index.filter(|index| *index <= INSERT_SLOTS).ok_or_else(|| error(format!("{name} must be A through K")))
    };
    let from = parse("from")?;
    let to = parse("to")?;
    let before = params.get("placement").and_then(Value::as_str) == Some("before");
    let copy = params.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let before_source = slots(engine.session().track(source).ok_or_else(|| error("Source track is missing"))?);
    let before_target = slots(engine.session().track(target).ok_or_else(|| error("Destination track is missing"))?);
    let device = before_source[from].clone().ok_or_else(|| error("The source insert is empty"))?;
    if params.get("expected_plugin").and_then(Value::as_str).is_some_and(|expected| expected != device.insert.plugin) { return Err(error("The source insert changed during the drag. Try again.")); }
    if from == to && source == target && !copy { return Ok(json!({"track": target.0, "slot": to, "reset": []})); }
    let mut destination_slot = to;
    let mut source_slots = before_source.clone();
    let mut target_slots = before_target.clone();
    if source == target && !copy {
        let destination = to.saturating_sub(usize::from(before && from < to));
        destination_slot = destination;
        let device = source_slots.remove(from);
        source_slots.insert(destination, device);
        target_slots = source_slots.clone();
    } else if before {
        let empty = target_slots.iter().take(INSERT_SLOTS).position(Option::is_none).ok_or_else(|| error("This track has no free insert"))?;
        target_slots.remove(empty);
        let destination = to.saturating_sub(usize::from(empty < to));
        destination_slot = destination;
        target_slots.insert(destination, Some(device));
        if !copy { source_slots[from] = None; }
        if source == target { source_slots = target_slots.clone(); }
    } else {
        if copy && target_slots[to].is_some() { return Err(error("Drop a copied device onto an empty insert")); }
        if copy { target_slots[to] = Some(device); }
        else { source_slots[from] = target_slots[to].take(); target_slots[to] = Some(device); }
        if source == target { source_slots = target_slots.clone(); }
    }
    let mut staged = engine.session().clone();
    if source != target { rebuild(engine.session(), staged.track_mut(source).ok_or_else(|| error("Source track is missing"))?, &source_slots)?; }
    rebuild(engine.session(), staged.track_mut(target).ok_or_else(|| error("Destination track is missing"))?, &target_slots)?;
    if source != target { route_values(engine.session(), &mut staged, source, &source_slots); }
    route_values(engine.session(), &mut staged, target, &target_slots);
    let mut reset = Vec::new();
    if source != target { reset_slots(source, &before_source, &source_slots, &mut reset); }
    reset_slots(target, &before_target, &target_slots, &mut reset);
    *engine.session_mut() = staged;
    Ok(json!({"track": target.0, "slot": destination_slot, "copy": copy, "reset": reset}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn moving_an_effect_preserves_its_automation_and_plugin_bus_assignments() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let bus = engine.session_mut().add_bus("Key", soundcraft_model::ChannelFormat::Stereo);
        let mut effect = Insert::new("gain");
        effect.params.insert("gain".into(), -9.0);
        engine.session_mut().track_mut(track).unwrap().mixer.inserts[0] = Some(effect.clone());
        let mut lane = soundcraft_model::AutomationLane::new(AutoParam::Plugin { slot: 0, param: "gain".into() });
        lane.set_point(1200, -6.0);
        engine.session_mut().track_mut(track).unwrap().automation.push(lane);
        engine.session_mut().edit.values.insert(format!("plugin_sidechain.{}.0", track.0), bus.0 as f64);
        engine.execute("mix.insert_move", &json!({"track": track.0, "from": 0, "to": 2})).unwrap();
        let moved = engine.session().track(track).unwrap();
        assert_eq!(moved.mixer.inserts[2].as_ref(), Some(&effect));
        assert!(moved.mixer.inserts[0].is_none());
        assert_eq!(moved.lane(&AutoParam::Plugin { slot: 2, param: "gain".into() }).unwrap().points[0].value, -6.0);
        assert_eq!(soundcraft_mix::plugin_sidechain(engine.session(), track, 2), Some(bus));
        assert_eq!(soundcraft_mix::plugin_sidechain(engine.session(), track, 0), None);
    }

    #[test]
    fn copying_between_tracks_keeps_the_source_and_copies_automation() {
        let mut engine = Engine::default();
        let source = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let target = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        engine.session_mut().track_mut(source).unwrap().mixer.inserts[0] = Some(Insert::new("gain"));
        let mut lane = soundcraft_model::AutomationLane::new(AutoParam::Plugin { slot: 0, param: "gain".into() });
        lane.set_point(0, -12.0);
        engine.session_mut().track_mut(source).unwrap().automation.push(lane);
        engine.execute("mix.insert_move", &json!({"from_track": source.0, "track": target.0, "from": 0, "to": 1, "copy": true})).unwrap();
        assert!(engine.session().track(source).unwrap().mixer.inserts[0].is_some());
        let target = engine.session().track(target).unwrap();
        assert!(target.mixer.inserts[1].is_some());
        assert_eq!(target.lane(&AutoParam::Plugin { slot: 1, param: "gain".into() }).unwrap().points[0].value, -12.0);
    }

    #[test]
    fn an_invalid_copy_changes_neither_track() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let source = engine.session_mut().track_mut(track).unwrap();
        source.mixer.inserts[0] = Some(Insert::new("gain"));
        source.mixer.inserts[1] = Some(Insert::new("trim"));
        let before = engine.session().to_json().unwrap();
        assert!(engine.execute("mix.insert_move", &json!({"track": track.0, "from": 0, "to": 1, "copy": true})).is_err());
        assert_eq!(engine.session().to_json().unwrap(), before);
    }
    #[test]
    fn a_plugin_key_route_cannot_introduce_a_feedback_cycle() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let bus = engine.session_mut().add_bus("Loop", soundcraft_model::ChannelFormat::Stereo);
        let source = engine.session_mut().track_mut(track).unwrap();
        source.mixer.inserts[0] = Some(Insert::new("gain"));
        source.mixer.output = soundcraft_model::Route::Bus(bus);
        let before = engine.session().to_json().unwrap();
        assert!(engine.execute("mix.plugin_routing", &json!({"track": track.0, "slot": 0, "sidechain": bus.0})).is_err());
        assert_eq!(engine.session().to_json().unwrap(), before);
    }

}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod chain_insertion_tests {
    use super::*;

    #[test]
    fn inserting_a_block_before_existing_devices_shifts_automation_and_undoes_once() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let mut first = Insert::new("gain");
        first.params.insert("gain".into(), -9.0);
        let mut second = Insert::new("gain");
        second.params.insert("gain".into(), -12.0);
        let model = engine.session_mut().track_mut(track).unwrap();
        model.mixer.inserts[0] = Some(first.clone());
        model.mixer.inserts[1] = Some(second.clone());
        let mut lane = soundcraft_model::AutomationLane::new(AutoParam::Plugin { slot: 0, param: "gain".into() });
        lane.set_point(0, -9.0);
        model.automation.push(lane);
        let before = engine.session().to_json().unwrap();
        let result = engine.execute("mix.insert_before", &json!({"track": track.0, "before": 0, "plugin": "gain"})).unwrap();
        assert_eq!(result["slot"], 0);
        let model = engine.session().track(track).unwrap();
        assert_eq!(model.mixer.inserts[1].as_ref(), Some(&first));
        assert_eq!(model.mixer.inserts[2].as_ref(), Some(&second));
        assert!(model.lane(&AutoParam::Plugin { slot: 1, param: "gain".into() }).is_some());
        assert!(engine.undo());
        assert_eq!(engine.session().to_json().unwrap(), before);
    }

    #[test]
    fn a_cross_track_gap_drop_inserts_without_swapping_the_destination_device() {
        let mut engine = Engine::default();
        let source = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let target = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let mut effect = Insert::new("gain");
        effect.params.insert("gain".into(), -3.0);
        engine.session_mut().track_mut(source).unwrap().mixer.inserts[0] = Some(effect.clone());
        engine.session_mut().track_mut(target).unwrap().mixer.inserts[0] = Some(Insert::new("gain"));
        engine.execute("mix.insert_move", &json!({"from_track": source.0, "track": target.0, "from": 0, "to": 0, "placement": "before"})).unwrap();
        assert!(engine.session().track(source).unwrap().mixer.inserts[0].is_none());
        let target = engine.session().track(target).unwrap();
        assert_eq!(target.mixer.inserts[0].as_ref(), Some(&effect));
        assert!(target.mixer.inserts[1].is_some());
    }

    #[test]
    fn copying_before_a_device_retains_the_source_and_reports_the_new_slot() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        engine.session_mut().track_mut(track).unwrap().mixer.inserts[0] = Some(Insert::new("gain"));
        let result = engine.execute("mix.insert_move", &json!({"track": track.0, "from": 0, "to": 1, "placement": "before", "copy": true})).unwrap();
        assert_eq!(result["slot"], 1);
        let model = engine.session().track(track).unwrap();
        assert!(model.mixer.inserts[0].is_some());
        assert!(model.mixer.inserts[1].is_some());
    }
}
