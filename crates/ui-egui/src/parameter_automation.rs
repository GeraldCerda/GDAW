use soundcraft_engine::Engine;
use soundcraft_model::{AutoParam, AutomationLane, AutomationMode, TrackId};
use soundcraft_time::Samples;
use std::collections::{BTreeMap, BTreeSet};

type Key = (TrackId, AutoParam);

struct Gesture {
    baseline: AutomationLane,
    value: f32,
    last: Samples,
}

#[derive(Default)]
pub struct ParameterAutomation {
    values: BTreeMap<Key, f32>,
    gestures: BTreeMap<Key, Gesture>,
    native: BTreeSet<Key>,
}

fn name(parameter: &AutoParam) -> String {
    match parameter {
        AutoParam::Volume => "volume".into(),
        AutoParam::Pan(index) => format!("pan{}", index + 1),
        AutoParam::Mute => "mute".into(),
        AutoParam::SendLevel(slot) => format!("send_{}_level", char::from(b'A'.saturating_add(*slot))),
        AutoParam::SendPan(slot) => format!("send_{}_pan", char::from(b'A'.saturating_add(*slot))),
        AutoParam::SendMute(slot) => format!("send_{}_mute", char::from(b'A'.saturating_add(*slot))),
        AutoParam::Plugin { slot, param } => format!("plugin:{}:{param}", char::from(b'A'.saturating_add(*slot))),
    }
}

fn point(engine: &mut Engine, key: &Key, at: Samples, from: Option<Samples>, value: f32) {
    let mut params = serde_json::json!({"track": key.0.0, "param": name(&key.1), "at": at, "value": value});
    if let Some(from) = from { params["from"] = from.into(); }
    let _ = engine.execute_merged("automation.set_point", &params, "automation_pass");
}

impl ParameterAutomation {
    pub fn step(&mut self, engine: &mut Engine, at: Samples, playing: bool, pointer: bool, events: Vec<(TrackId, usize, String, bool)>) {
        let mut values = BTreeMap::new();
        for track in &engine.session().tracks {
            for (index, value) in track.mixer.pan.iter().enumerate() { values.insert((track.id, AutoParam::Pan(index as u8)), *value); }
            values.insert((track.id, AutoParam::Mute), if track.mixer.mute { 1.0 } else { 0.0 });
            for (slot, send) in track.mixer.sends.iter().enumerate() {
                if let Some(send) = send {
                    values.insert((track.id, AutoParam::SendLevel(slot as u8)), send.level_db);
                    values.insert((track.id, AutoParam::SendPan(slot as u8)), send.pan);
                    values.insert((track.id, AutoParam::SendMute(slot as u8)), if send.mute { 1.0 } else { 0.0 });
                }
            }
            for (slot, insert) in track.mixer.inserts.iter().enumerate() {
                if let Some(insert) = insert { for (parameter, value) in &insert.params { values.insert((track.id, AutoParam::Plugin { slot: slot as u8, param: parameter.clone() }), *value); } }
            }
            if let Some(instrument) = &track.instrument {
                let slot = track.instrument_insert_slot().unwrap_or(0) as u8;
                for (parameter, value) in &instrument.params { values.insert((track.id, AutoParam::Plugin { slot, param: parameter.clone() }), *value); }
            }
        }
        for (track, slot, parameter, begin) in events {
            let slot = if slot == soundcraft_mix::INSTRUMENT_SLOT { engine.session().track(track).and_then(|track| track.instrument_insert_slot()).unwrap_or(0) } else { slot };
            let Ok(slot) = u8::try_from(slot) else { continue };
            let key = (track, AutoParam::Plugin { slot, param: parameter });
            if begin { self.native.insert(key); } else { self.native.remove(&key); }
        }
        if !playing { self.gestures.clear(); self.native.clear(); self.values = values; return; }
        for (key, value) in &values {
            let Some(track) = engine.session().track(key.0) else { continue };
            let mode = track.mixer.automation_mode;
            if !matches!(mode, AutomationMode::Write | AutomationMode::Touch | AutomationMode::Latch | AutomationMode::TouchLatch) { continue; }
            let changed = self.values.get(key).is_some_and(|old| old.to_bits() != value.to_bits());
            let armed_write = mode == AutomationMode::Write && track.lane(&key.1).is_some_and(|lane| lane.enabled);
            if !self.gestures.contains_key(key) && (changed || armed_write) {
                let initial = self.values.get(key).copied().unwrap_or(*value);
                let mut baseline = track.lane(&key.1).cloned().unwrap_or_else(|| AutomationLane::new(key.1.clone()));
                if baseline.points.is_empty() { baseline.set_point(0, initial); }
                self.gestures.insert(key.clone(), Gesture { baseline, value: *value, last: i64::MIN });
            }
        }
        let step = engine.session().sample_rate.samples(0.02);
        let ramp = engine.session().sample_rate.samples(0.1);
        let mut remove = Vec::new();
        for (key, gesture) in &mut self.gestures {
            let Some(track) = engine.session().track(key.0) else { remove.push(key.clone()); continue };
            let mode = track.mixer.automation_mode;
            if !matches!(mode, AutomationMode::Write | AutomationMode::Touch | AutomationMode::Latch | AutomationMode::TouchLatch) { remove.push(key.clone()); continue; }
            let value = values.get(key).copied().unwrap_or(gesture.value);
            let touched = self.native.contains(key) || pointer;
            let discrete = matches!(key.1, AutoParam::Mute | AutoParam::SendMute(_));
            let release = mode == AutomationMode::Touch && !touched && !discrete;
            let changed = value.to_bits() != gesture.value.to_bits();
            if gesture.last == i64::MIN || changed || at < gesture.last || at.saturating_sub(gesture.last) >= step || release {
                let from = (gesture.last != i64::MIN && at >= gesture.last).then_some(gesture.last);
                point(engine, key, at, from, value);
                gesture.value = value;
                gesture.last = at;
            }
            if release {
                let end = at.saturating_add(ramp);
                point(engine, key, end, Some(at), gesture.baseline.value_at(end, value));
                remove.push(key.clone());
            }
        }
        for key in remove { self.gestures.remove(&key); }
        self.values = values;
    }
}
