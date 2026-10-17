use serde_json::json;
use soundcraft_engine::{Engine, TransportRequest};
use soundcraft_model::{ChannelFormat, Insert, MonitorMode, TrackKind};
use soundcraft_time::Range;

#[test]
fn loop_brace_is_independent_of_selection_and_recording_protects_it() {
    let mut engine = Engine::default();
    engine.execute("edit.select", &json!({"start": 1_000, "end": 5_000})).unwrap();
    engine.execute("transport.loop", &json!({"selection": true})).unwrap();
    engine.execute("edit.select", &json!({"start": 9_000, "end": 10_000})).unwrap();
    assert_eq!(engine.session().edit.arrangement_loop, Some(Range::new(1_000, 5_000)));
    engine.execute("transport.loop", &json!({"enabled": false})).unwrap();
    assert!(!engine.session().edit.loop_playback);
    assert!(!engine.session().edit.loop_record);
    assert_eq!(engine.transport_requests.last(), Some(&TransportRequest::Loop(None)));
    engine.transport.recording = true;
    assert!(engine.execute("transport.loop", &json!({"start": 100, "end": 200})).is_err());
    assert_eq!(engine.session().edit.arrangement_loop, Some(Range::new(1_000, 5_000)));
}

#[test]
fn record_can_be_armed_without_starting_playback() {
    let mut engine = Engine::default();
    engine.execute("transport.record", &json!({"start_playback": false})).unwrap();
    assert_eq!(engine.transport_requests, vec![TransportRequest::RecordArm]);
}

#[test]
fn device_mapping_is_validated_saved_and_follows_a_move() {
    let mut engine = Engine::default();
    let track = engine.session_mut().add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
    engine.session_mut().track_mut(track).unwrap().mixer.inserts[0] = Some(Insert::new("gain"));
    engine.execute("mix.device_controls", &json!({"track": track.0, "slot": 0, "x": "gain", "parameters": ["gain", "gain"]})).unwrap();
    assert!(engine.execute("mix.device_controls", &json!({"track": track.0, "slot": 0, "y": "gain"})).is_err());
    assert!(engine.execute("mix.device_controls", &json!({"track": track.0, "slot": 0, "x": "missing"})).is_err());
    engine.execute("mix.insert_move", &json!({"track": track.0, "from": 0, "to": 2})).unwrap();
    let insert = engine.session().track(track).unwrap().mixer.inserts[2].as_ref().unwrap();
    assert_eq!(insert.controls.x.as_deref(), Some("gain"));
    assert_eq!(insert.controls.parameters, vec!["gain"]);
    let saved = serde_json::to_string(insert).unwrap();
    assert_eq!(serde_json::from_str::<Insert>(&saved).unwrap(), *insert);
}

#[test]
fn exclusive_arm_and_monitor_modes_are_explicit_and_undoable() {
    let mut engine = Engine::default();
    let first = engine.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    let second = engine.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    engine.execute("mix.record_arm", &json!({"track": first.0, "value": true})).unwrap();
    engine.execute("mix.record_arm", &json!({"track": second.0, "value": true, "exclusive": true})).unwrap();
    assert!(!engine.session().track(first).unwrap().mixer.record_arm);
    assert!(engine.session().track(second).unwrap().mixer.record_arm);
    engine.execute("mix.monitor_mode", &json!({"track": second.0, "mode": "auto"})).unwrap();
    let mixer = &engine.session().track(second).unwrap().mixer;
    assert_eq!(mixer.monitor_mode, Some(MonitorMode::Auto));
    assert!(mixer.monitors_input(false, false, false));
    assert!(!mixer.monitors_input(false, true, true));
    assert!(mixer.monitors_input(true, true, true));
    engine.execute("mix.monitor_mode", &json!({"track": second.0, "mode": "off"})).unwrap();
    assert!(!engine.session().track(second).unwrap().mixer.monitors_input(true, true, false));
    engine.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(engine.session().track(second).unwrap().mixer.monitor_mode, Some(MonitorMode::Auto));
}

#[test]
fn stale_replacement_target_keeps_the_original_device() {
    let mut engine = Engine::default();
    let track = engine.session_mut().add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
    engine.session_mut().track_mut(track).unwrap().mixer.inserts[0] = Some(Insert::new("gain"));
    let before = engine.session().clone();
    assert!(engine.execute("mix.device_replace", &json!({"track": track.0, "slot": 0, "plugin": "eq7", "expected_plugin": "comp"})).is_err());
    assert_eq!(engine.session(), &before);
}
