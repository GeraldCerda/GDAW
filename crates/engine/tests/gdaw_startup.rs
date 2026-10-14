use serde_json::json;
use soundcraft_engine::{Engine, starter_session};
use soundcraft_model::{ChannelFormat, Route, TrackKind};
use soundcraft_time::SampleRate;

#[test]
fn startup_has_a_stereo_drum_rack_and_a_mono_recording_track() {
    let session = starter_session("Startup", SampleRate::HZ_48000);
    assert_eq!(session.tracks.len(), 2);
    let midi = session.track_by_name("MIDI 1").unwrap();
    assert_eq!(midi.kind, TrackKind::Instrument);
    assert_eq!(midi.format, ChannelFormat::Stereo);
    assert_eq!(midi.instrument.as_ref().unwrap().plugin, "drum_rack");
    let audio = session.track_by_name("Audio 1").unwrap();
    assert_eq!(audio.kind, TrackKind::Audio);
    assert_eq!(audio.format, ChannelFormat::Mono);
    assert_eq!(audio.mixer.input, Route::Hardware("In 1".into()));
    assert_eq!(audio.mixer.output, Route::Main);
    assert!(session.edit.flag("edit_view.io"));
    assert!(session.edit.flag("edit_view.inserts_ae"));
    assert!(session.edit.flag("edit_view.sends_ae"));
    assert_eq!(session.edit.selected_tracks, vec![midi.id]);
}

#[test]
fn the_gdaw_template_and_explicit_blank_template_stay_distinct() {
    let mut engine = Engine::default();
    engine.execute("session.new", &json!({"template": "gdaw", "sample_rate": 96_000})).unwrap();
    assert_eq!(engine.session().tracks.len(), 2);
    assert_eq!(engine.session().sample_rate.hz(), 96_000);
    assert!(!engine.is_dirty());
    engine.execute("session.new", &json!({"template": "blank"})).unwrap();
    assert!(engine.session().tracks.is_empty());
}

#[test]
fn startup_grid_exposes_eighth_notes_without_changing_zoom() {
    let session = starter_session("Grid", SampleRate::HZ_48000);
    assert_eq!(session.edit.grid.label(), "1/8");
    assert!(session.edit.grid_lines);
    assert_eq!(session.edit.edit_mode, soundcraft_model::EditMode::Grid);
    let pixels = session.edit.grid.step_samples(0, session.sample_rate, &session.tempo, session.frame_rate) as f64 / session.edit.zoom.samples_per_px;
    assert!(pixels >= 2.0);
}
