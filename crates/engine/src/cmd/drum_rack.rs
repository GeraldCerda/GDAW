use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_model::{Insert, SourceId, TrackKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mix.drum_pad_effect", "Edit Drum Pad Effect", [], None, "{track?, note: 36..51, slot?: 0..9, action: insert|remove|bypass|param, plugin?, params?, param?, value?}", has_selection, effect),
        cmd!("mix.drum_kit_load", "Load Drum Kit", [], None, "{track?, paths: [audio paths], replace?: false, skip_invalid?: false}", has_selection, kit),
        cmd!("mix.drum_pad_load", "Load Drum Pad", [], None, "{track?, note: 36..51, path?|source?}", has_selection, load),
        cmd!(noundo "mix.drum_pad_audition", "Audition Drum Pad", [], None, "{track?, note: 36..51}", has_selection, audition),
        cmd!("mix.drum_pad_clear", "Clear Drum Pad", [], None, "{track?, note: 36..51}", has_selection, clear),
        cmd!("mix.drum_pad_param", "Set Drum Pad Parameter", [], None, "{track?, note: 36..51, param: gain|pan|tune|mute, value}", has_selection, param),
    ]
}

fn target(engine: &Engine, params: &Value, command: &str) -> Result<(TrackId, u8, Insert)> {
    let note = params.get("note").and_then(Value::as_u64).filter(|note| (36..=51).contains(note)).ok_or_else(|| bad(command, "note must be 36..51"))? as u8;
    let track = tracks_required(engine, command, params)?.first().copied().ok_or_else(|| bad(command, "no track"))?;
    let insert = engine.session().track(track).filter(|track| track.kind == TrackKind::Instrument)
        .and_then(|track| track.instrument.as_ref()).filter(|insert| insert.plugin == "drum_rack")
        .cloned().ok_or_else(|| bad(command, "the track must have a Drum Rack instrument"))?;
    Ok((track, note, insert))
}

fn kit(engine: &mut Engine, params: &Value) -> Result<Value> {
    let paths = params.get("paths").and_then(Value::as_array).filter(|paths| !paths.is_empty() && paths.len() <= 16)
        .ok_or_else(|| bad("mix.drum_kit_load", "provide 1..16 audio paths"))?;
    let paths = paths.iter().map(|path| path.as_str().filter(|path| !path.is_empty()).ok_or_else(|| bad("mix.drum_kit_load", "each path must be a nonempty string"))).collect::<Result<Vec<_>>>()?;
    let mut target_params = params.clone();
    if let Some(object) = target_params.as_object_mut() {
        object.insert("note".into(), json!(36));
    }
    let (track, _, mut insert) = target(engine, &target_params, "mix.drum_kit_load")?;
    let replace = bool_or(params, "replace", false);
    let skip_invalid = bool_or(params, "skip_invalid", false);
    let available: Vec<u8> = if skip_invalid {
        (36..=51).filter(|note| replace || !insert.pad_samples.contains_key(note)).collect()
    } else {
        Vec::new()
    };
    if replace {
        insert.pad_samples.clear();
        if let Some(track) = engine.session_mut().track_mut(track) {
            track.instrument = Some(insert);
        }
    }
    let mut loaded = 0usize;
    let mut skipped = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let note = if skip_invalid {
            let Some(note) = available.get(loaded).copied() else { break };
            note
        } else {
            36 + index as u8
        };
        if !replace && engine.session().track(track).and_then(|track| track.instrument.as_ref()).is_some_and(|insert| insert.pad_samples.contains_key(&note)) {
            continue;
        }
        match load(engine, &json!({"track": track.0, "note": note, "path": path})) {
            Ok(_) => loaded += 1,
            Err(error) if skip_invalid => skipped.push(error.to_string()),
            Err(error) => return Err(error),
        }
    }
    if loaded == 0 && !skipped.is_empty() {
        return Err(bad("mix.drum_kit_load", format!("no samples could be loaded: {}", skipped.join("; "))));
    }
    Ok(json!({"track": track, "loaded": loaded, "skipped": skipped}))
}

fn load(engine: &mut Engine, params: &Value) -> Result<Value> {
    let command = "mix.drum_pad_load";
    let (track, note, mut insert) = target(engine, params, command)?;
    let path = str_param(params, "path");
    let source = params.get("source").and_then(Value::as_u64).map(SourceId);
    if path.is_some() == source.is_some() {
        return Err(bad(command, "provide exactly one of path or source"));
    }
    let source = if let Some(source) = source {
        if !engine.session().pool.contains(source) {
            return Err(bad(command, "sample source is missing or unavailable"));
        }
        source
    } else {
        let path = path.ok_or_else(|| bad(command, "path required"))?;
        let bytes = crate::io::read_file(path)?;
        let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("Sample");
        super::sampler::attach(engine, track, name, &bytes, Some(path), Vec::new())?;
        engine.session().track(track).and_then(|track| track.instrument.as_ref()).and_then(|insert| insert.sample)
            .ok_or_else(|| bad(command, "sample could not be attached"))?
    };
    insert.pad_samples.insert(note, source);
    if let Some(track) = engine.session_mut().track_mut(track) {
        track.instrument = Some(insert);
    }
    Ok(json!({"track": track, "note": note, "source": source}))
}

fn audition(engine: &mut Engine, params: &Value) -> Result<Value> {
    let (track, note, insert) = target(engine, params, "mix.drum_pad_audition")?;
    if !insert.pad_samples.contains_key(&note) {
        return Err(bad("mix.drum_pad_audition", "the pad has no sample"));
    }
    engine.transport_requests.push(crate::TransportRequest::Audition(track, note));
    Ok(json!({"track": track, "note": note}))
}

fn clear(engine: &mut Engine, params: &Value) -> Result<Value> {
    let (track, note, mut insert) = target(engine, params, "mix.drum_pad_clear")?;
    let source = insert.pad_samples.remove(&note);
    if let Some(track) = engine.session_mut().track_mut(track) {
        track.instrument = Some(insert);
    }
    Ok(json!({"track": track, "note": note, "source": source}))
}

fn param(engine: &mut Engine, params: &Value) -> Result<Value> {
    let command = "mix.drum_pad_param";
    let (track, note, mut insert) = target(engine, params, command)?;
    let name = str_param(params, "param").ok_or_else(|| bad(command, "param required"))?;
    let value = params.get("value").and_then(Value::as_f64).filter(|value| value.is_finite()).ok_or_else(|| bad(command, "finite value required"))?;
    let value = match name {
        "gain" => value.clamp(-48.0, 12.0),
        "pan" => value.clamp(-1.0, 1.0),
        "tune" => value.clamp(-48.0, 48.0),
        "mute" => value.round().clamp(0.0, 1.0),
        _ => return Err(bad(command, "unknown pad parameter")),
    };
    insert.params.insert(format!("pad{note}.{name}"), value as f32);
    if let Some(track) = engine.session_mut().track_mut(track) {
        track.instrument = Some(insert);
    }
    Ok(json!({"track": track, "note": note, "param": name, "value": value}))
}

fn effect(engine: &mut Engine, params: &Value) -> Result<Value> {
    let command = "mix.drum_pad_effect";
    let (track, note, mut instrument) = target(engine, params, command)?;
    let action = str_param(params, "action").ok_or_else(|| bad(command, "action required"))?;
    let slots = instrument.pad_effects.entry(note).or_insert_with(|| std::array::from_fn(|_| None));
    let slot = match params.get("slot") {
        Some(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()).filter(|slot| *slot < slots.len())
            .ok_or_else(|| bad(command, "slot must be 0..9"))?,
        None if action == "insert" => slots.iter().position(Option::is_none).ok_or_else(|| bad(command, "all pad effect slots are in use"))?,
        None => return Err(bad(command, "slot required")),
    };
    match action {
        "insert" => {
            let plugin = str_param(params, "plugin").ok_or_else(|| bad(command, "plugin required"))?;
            let info = super::clap::plugin_info(plugin).ok_or_else(|| bad(command, "unknown effect"))?;
            if info.is_instrument || soundcraft_dsp::is_generator(plugin) {
                return Err(bad(command, "choose an audio effect for this pad"));
            }
            let mut insert = Insert::new(plugin);
            for parameter in info.params { insert.params.insert(parameter.id.into(), parameter.default); }
            if let Some(values) = params.get("params") {
                let values = values.as_object().ok_or_else(|| bad(command, "params must be an object"))?;
                for (name, value) in values {
                    let parameter = info.param(name).ok_or_else(|| bad(command, format!("unknown parameter {name}")))?;
                    let value = value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| bad(command, "finite parameter value required"))?;
                    insert.params.insert(name.clone(), parameter.clamp(value as f32));
                }
            }
            if let Some(target) = slots.get_mut(slot) { *target = Some(insert); }
        }
        "remove" => {
            if let Some(target) = slots.get_mut(slot) { *target = None; }
        }
        "bypass" | "param" => {
            let insert = slots.get_mut(slot).and_then(Option::as_mut).ok_or_else(|| bad(command, "empty pad effect slot"))?;
            if action == "bypass" {
                insert.bypass = match params.get("value") {
                    None => !insert.bypass,
                    Some(value) => value.as_bool().ok_or_else(|| bad(command, "bypass value must be boolean"))?,
                };
            } else {
                let name = str_param(params, "param").ok_or_else(|| bad(command, "param required"))?;
                let value = params.get("value").and_then(Value::as_f64).filter(|value| value.is_finite())
                    .ok_or_else(|| bad(command, "finite parameter value required"))?;
                let parameter = super::clap::plugin_info(&insert.plugin).and_then(|info| info.param(name))
                    .ok_or_else(|| bad(command, "unknown effect parameter"))?;
                insert.params.insert(name.into(), parameter.clamp(value as f32));
            }
        }
        _ => return Err(bad(command, "unknown pad effect action")),
    }
    instrument.pad_effects.retain(|_, slots| slots.iter().any(Option::is_some));
    if let Some(track) = engine.session_mut().track_mut(track) { track.instrument = Some(instrument); }
    Ok(json!({"track":track,"note":note,"slot":slot}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, FileFormat};
    use soundcraft_model::{ChannelFormat, Clip};
    use soundcraft_midi::{Note, Sequence};
    use soundcraft_time::Range;

    #[test]
    fn pad_assignment_renders_saves_and_undoes_without_replacing_the_rack() {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Rack"));
        engine.execute("mix.instrument", &json!({"track": track.0, "plugin": "drum_rack"})).unwrap();
        let source = crate::io::add_source(engine.session_mut(), "Pad", AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 4800]] }, None, FileFormat::Wav);
        engine.execute("mix.drum_pad_load", &json!({"track": track.0, "note": 36, "source": source.0})).unwrap();
        let session = engine.session_mut();
        let clip = session.new_clip_id();
        let sequence = Sequence { notes: vec![Note { pitch: 36, velocity: 127, release_velocity: 64, channel: 0, start: 0, length: 480 }], ctrls: Vec::new() };
        crate::edit::place_clip(session, track, Clip::midi(clip, "Beat", 0, 4800, sequence));
        let audio = soundcraft_mix::render_range(engine.session(), Range::new(0, 4800), 128);
        assert!(audio.iter().flatten().any(|value| value.abs() > 0.1));
        let folder = std::env::temp_dir().join(format!("gdaw-pad-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("Rack.scraft").to_string_lossy().into_owned();
        engine.execute("session.save_as", &json!({"path": path})).unwrap();
        let mut reopened_engine = Engine::default();
        reopened_engine.execute("session.open", &json!({"path": path})).unwrap();
        let reopened_audio = soundcraft_mix::render_range(reopened_engine.session(), Range::new(0, 4800), 128);
        assert!(reopened_audio.iter().flatten().any(|value| value.abs() > 0.1));
        let mut mixer = soundcraft_mix::MixEngine::new(48_000.0, 128);
        mixer.monitor_only = true;
        mixer.audition(engine.session(), track, 36);
        let mut preview = vec![vec![0.0; 128]; 2];
        mixer.render(engine.session(), 0, 128, &mut preview);
        mixer.render(engine.session(), 0, 128, &mut preview);
        assert!(preview.iter().flatten().any(|value| value.abs() > 0.1));
        engine.execute("mix.drum_pad_param", &json!({"track": track.0, "note": 36, "param": "pan", "value": 1.0})).unwrap();
        mixer.audition(engine.session(), track, 36);
        mixer.render(engine.session(), 0, 128, &mut preview);
        assert!(preview[0].iter().all(|value| value.abs() < 1e-6));
        assert!(preview[1].iter().any(|value| value.abs() > 0.1));
        engine.execute("edit.undo", &json!({})).unwrap();
        mixer.audition(engine.session(), track, 36);
        mixer.render(engine.session(), 0, 128, &mut preview);
        assert!(preview[0].iter().any(|value| value.abs() > 0.1));
        mixer.end_audition();
        mixer.render(engine.session(), 0, 128, &mut preview);
        assert!(preview.iter().flatten().all(|value| value.abs() < 1e-6));
        let saved = serde_json::to_string(engine.session()).unwrap();
        let reopened: soundcraft_model::Session = serde_json::from_str(&saved).unwrap();
        assert_eq!(reopened.track(track).unwrap().instrument.as_ref().unwrap().pad_samples.get(&36), Some(&source));
        engine.execute("mix.drum_pad_clear", &json!({"track": track.0, "note": 36})).unwrap();
        let audio = soundcraft_mix::render_range(engine.session(), Range::new(0, 4800), 128);
        assert!(audio.iter().flatten().all(|value| value.abs() < 1e-6));
        engine.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(engine.session().track(track).unwrap().instrument.as_ref().unwrap().pad_samples.get(&36), Some(&source));
        assert!(engine.execute("mix.drum_pad_load", &json!({"track": track.0, "note": 200, "source": source.0})).is_err());
        assert!(engine.execute("clip.clear", &json!({"sources": [source.0], "with_clips": true})).is_err());
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[cfg(test)]
mod kit_tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat};

    #[test]
    fn kit_loading_is_one_undo_step_and_failed_replacements_restore_the_pads() {
        let mut engine = Engine::new(crate::starter_session("Kit", soundcraft_time::SampleRate::HZ_48000));
        let track = engine.session().track_by_name("MIDI 1").unwrap().id;
        let folder = std::env::temp_dir().join(format!("gdaw-kit-load-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let sample = folder.join("sample.wav");
        let audio = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 480]] };
        let options = EncodeOptions { format: FileFormat::Wav, bit_depth: BitDepth::Int16, dither: false, bwf: None };
        std::fs::write(&sample, soundcraft_audio_io::encode(&audio, &options).unwrap()).unwrap();
        let before = engine.session().clone();
        let steps = engine.undo_history().len();
        engine.execute("mix.drum_kit_load", &json!({"track": track.0, "paths": [sample, sample]})).unwrap();
        assert_eq!(engine.session().track(track).unwrap().instrument.as_ref().unwrap().pad_samples.len(), 2);
        assert_eq!(engine.undo_history().len(), steps + 1);
        let loaded = engine.session().clone();
        assert!(engine.execute("mix.drum_kit_load", &json!({"track": track.0, "paths": [sample, folder.join("missing.wav")], "replace": true})).is_err());
        assert_eq!(engine.session(), &loaded);
        engine.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(engine.session(), &before);
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[cfg(test)]
mod tolerant_kit_tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat};

    #[test]
    fn random_kits_skip_bad_files_and_keep_atomic_undo_and_failed_replacement_rollback() {
        let mut engine = Engine::new(crate::starter_session("Kit", soundcraft_time::SampleRate::HZ_48000));
        let track = engine.session().track_by_name("MIDI 1").unwrap().id;
        let root = std::env::temp_dir().join(format!("gdaw-tolerant-kit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let good = root.join("good.wav");
        let bad = root.join("bad.wav");
        let audio = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 480]] };
        let options = EncodeOptions { format: FileFormat::Wav, bit_depth: BitDepth::Int16, dither: false, bwf: None };
        std::fs::write(&good, soundcraft_audio_io::encode(&audio, &options).unwrap()).unwrap();
        std::fs::write(&bad, b"unreadable sample").unwrap();
        let before = engine.session().clone();
        let steps = engine.undo_history().len();
        let result = engine.execute("mix.drum_kit_load", &json!({"track": track.0, "paths": [bad, good], "skip_invalid": true})).unwrap();
        assert_eq!(result["loaded"], 1);
        assert_eq!(result["skipped"].as_array().unwrap().len(), 1);
        let rack = engine.session().track(track).unwrap().instrument.as_ref().unwrap();
        assert!(rack.pad_samples.contains_key(&36));
        assert!(!rack.pad_samples.contains_key(&37));
        assert_eq!(engine.undo_history().len(), steps + 1);
        let loaded = engine.session().clone();
        engine.execute("mix.drum_kit_load", &json!({"track": track.0, "paths": [bad, good], "skip_invalid": true})).unwrap();
        let rack = engine.session().track(track).unwrap().instrument.as_ref().unwrap();
        assert_eq!(rack.pad_samples.get(&36), loaded.track(track).unwrap().instrument.as_ref().unwrap().pad_samples.get(&36));
        assert!(rack.pad_samples.contains_key(&37));
        engine.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(engine.session(), &loaded);
        assert!(engine.execute("mix.drum_kit_load", &json!({"track": track.0, "paths": [bad], "skip_invalid": true, "replace": true})).is_err());
        assert_eq!(engine.session(), &loaded);
        engine.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(engine.session(), &before);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod pad_effect_tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, FileFormat};
    use soundcraft_model::ChannelFormat;

    fn fixture() -> (Engine, TrackId) {
        let mut engine = Engine::default();
        let track = engine.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Rack"));
        engine.execute("mix.instrument", &json!({"track":track.0,"plugin":"drum_rack"})).unwrap();
        let source = crate::io::add_source(engine.session_mut(), "Pad",
            AudioBuffer { sample_rate:48_000, channels:vec![vec![0.25;48_000]] }, None, FileFormat::Wav);
        for note in [36,37] {
            engine.execute("mix.drum_pad_load",&json!({"track":track.0,"note":note,"source":source.0})).unwrap();
        }
        (engine,track)
    }

    fn preview(engine: &Engine, track: TrackId, note: u8) -> f32 {
        let mut mixer = soundcraft_mix::MixEngine::new(engine.session().sample_rate.as_f64() as f32,128);
        mixer.monitor_only = true;
        mixer.audition(engine.session(),track,note);
        let mut audio = vec![vec![0.0;128];2];
        for _ in 0..128 { mixer.render(engine.session(),0,128,&mut audio); }
        audio.iter().flatten().fold(0.0f32,|peak,sample|peak.max(sample.abs()))
    }

    #[test]
    fn effects_process_only_their_pad_bypass_remove_and_undo_restore_the_signal() {
        let (mut engine,track) = fixture();
        let dry = preview(&engine,track,36);
        engine.execute("mix.drum_pad_effect",&json!({"track":track.0,"note":36,"slot":4,"action":"insert","plugin":"gain","params":{"gain":-12.0}})).unwrap();
        let wet = preview(&engine,track,36);
        assert!(wet < dry * 0.27 && wet > dry * 0.23);
        assert!((preview(&engine,track,37)-dry).abs()<1e-5);
        engine.execute("mix.drum_pad_effect",&json!({"track":track.0,"note":36,"slot":4,"action":"bypass"})).unwrap();
        assert!((preview(&engine,track,36)-dry).abs()<1e-5);
        engine.execute("edit.undo",&json!({})).unwrap();
        assert!((preview(&engine,track,36)-wet).abs()<1e-5);
        engine.execute("mix.drum_pad_effect",&json!({"track":track.0,"note":36,"slot":4,"action":"remove"})).unwrap();
        assert!((preview(&engine,track,36)-dry).abs()<1e-5);
        engine.execute("edit.undo",&json!({})).unwrap();
        assert!((preview(&engine,track,36)-wet).abs()<1e-5);
    }

    #[test]
    fn pad_effects_round_trip_and_invalid_edits_preserve_the_session() {
        let (mut engine,track) = fixture();
        engine.execute("mix.drum_pad_effect",&json!({"track":track.0,"note":36,"slot":9,"action":"insert","plugin":"gain"})).unwrap();
        engine.execute("mix.drum_pad_effect",&json!({"track":track.0,"note":36,"slot":9,"action":"param","param":"gain","value":-9.0})).unwrap();
        let blob = soundcraft_model::b64::encode(b"pad editor state");
        let address = soundcraft_mix::drum_pad_slot(36,9).unwrap();
        engine.execute("mix.insert_state",&json!({"track":track.0,"slot":address,"state":blob})).unwrap();
        let saved = serde_json::to_string(engine.session()).unwrap();
        let reopened: soundcraft_model::Session = serde_json::from_str(&saved).unwrap();
        assert_eq!(reopened.track(track).unwrap().instrument,engine.session().track(track).unwrap().instrument);
        let before = engine.session().clone();
        for params in [
            json!({"note":35,"slot":0,"action":"insert","plugin":"gain"}),
            json!({"note":36,"slot":10,"action":"insert","plugin":"gain"}),
            json!({"note":36,"slot":0,"action":"insert","plugin":"sampler"}),
            json!({"note":36,"slot":0,"action":"insert","plugin":"unknown"}),
            json!({"note":36,"slot":0,"action":"insert","plugin":"gain","params":{"gain":"loud"}}),
            json!({"note":36,"slot":9,"action":"param","param":"unknown","value":0}),
            json!({"note":36,"slot":9,"action":"bypass","value":1}),
        ] {
            let mut params = params;
            params["track"] = json!(track.0);
            assert!(engine.execute("mix.drum_pad_effect",&params).is_err());
            assert_eq!(engine.session(),&before);
        }
    }
}
