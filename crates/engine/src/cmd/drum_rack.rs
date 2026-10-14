use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_model::{Insert, SourceId, TrackKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
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
