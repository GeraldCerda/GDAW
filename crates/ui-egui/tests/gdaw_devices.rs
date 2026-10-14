use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;
use soundcraft_ui_egui::{MainWindow, Services, SoundApp, devices::{BrowserItem, insert_item}};

fn harness() -> Harness<'static, SoundApp> {
    let engine = soundcraft_engine::Engine::new(soundcraft_engine::starter_session("G", soundcraft_engine::model::time::SampleRate::HZ_48000));
    let app = SoundApp::new(engine, None, Services::default());
    let mut harness = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(|ui, app: &mut SoundApp| {
        app.logic(&ui.ctx().clone());
        app.ui(ui);
    }, app);
    harness.run_steps(5);
    harness
}

#[test]
fn startup_shows_the_browser_drum_rack_and_recording_io_columns() {
    let harness = harness();
    assert!(harness.state().ui.show_browser);
    assert!(harness.state().ui.show_devices);
    assert!(harness.query_all_by_label_contains("BROWSER").count() > 0);
    assert!(harness.query_all_by_label_contains("Load").count() > 0);
    assert!(soundcraft_ui_egui::edit_window::edit_columns(harness.state().engine.session()).iter().any(|column| column.0 == "io"));
}

#[test]
fn command_equals_opens_a_separate_mixer_and_leaves_edit_in_place() {
    let mut harness = harness();
    let modifiers = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    harness.input_mut().events.push(Event::Key { key: Key::Equals, physical_key: None, pressed: true, repeat: false, modifiers });
    harness.step();
    assert!(harness.state().ui.mix_separate);
    assert_eq!(harness.state().ui.window, MainWindow::Edit);
    harness.state_mut().run("window.mix", json!({})).unwrap();
    assert!(harness.state().ui.mix_separate);
}

#[test]
fn browser_effects_append_to_the_selected_track_and_can_be_undone() {
    let mut harness = harness();
    let track = harness.state().engine.session().track_by_name("MIDI 1").unwrap().id;
    insert_item(harness.state_mut(), track, &BrowserItem::Plugin("compressor".into()));
    insert_item(harness.state_mut(), track, &BrowserItem::Plugin("eq_7band".into()));
    let inserts = &harness.state().engine.session().track(track).unwrap().mixer.inserts;
    assert_eq!(inserts[0].as_ref().unwrap().plugin, "compressor");
    assert_eq!(inserts[1].as_ref().unwrap().plugin, "eq_7band");
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(harness.state().engine.session().track(track).unwrap().mixer.inserts[1].is_none());
}

#[test]
fn plugin_browser_exposes_developer_effect_and_format_choices() {
    let harness = harness();
    assert!(harness.query_all_by_label_contains("Developer").count() > 0);
    assert!(harness.query_all_by_label_contains("Effect").count() > 0);
    assert!(harness.query_all_by_label_contains("All formats").count() > 0);
    assert_eq!(harness.state().browser.grouping, soundcraft_ui_egui::plugin_browser::PluginGrouping::Developer);
}

#[test]
fn headless_editor_does_not_start_native_plugin_discovery() {
    let harness = harness();
    assert!(!harness.state().browser.plugins_loaded);
    assert!(harness.state().browser.plugin_scan.is_none());
}

#[test]
fn mixer_toggle_hides_and_reopens_without_leaving_edit() {
    let mut harness = harness();
    for visible in [true, false, true] {
        harness.state_mut().run("window.toggle_mix_edit", json!({})).unwrap();
        assert_eq!(harness.state().ui.mix_separate, visible);
        assert_eq!(harness.state().ui.window, MainWindow::Edit);
    }
}

#[test]
fn drum_library_setting_round_trips_without_loading_samples_when_unassigned() {
    let mut harness = harness();
    let before = harness.state().engine.session().clone();
    soundcraft_ui_egui::devices::prepare_drum_library(harness.state_mut());
    assert_eq!(harness.state().engine.session(), &before);
    let root = std::env::temp_dir().join(format!("gdaw-empty-library-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    harness.state_mut().run("ui.drum_library", json!({"path": root.to_string_lossy()})).unwrap();
    soundcraft_ui_egui::devices::prepare_drum_library(harness.state_mut());
    assert!(harness.state().browser.library_files.is_empty());
    let saved = serde_json::to_value(&harness.state().ui).unwrap();
    let restored: soundcraft_ui_egui::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.drum_library, root.to_string_lossy());
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn both_starter_tracks_expose_five_separate_effect_inserts_and_sends() {
    let harness = harness();
    for letter in b'A'..=b'E' {
        assert_eq!(harness.query_all_by_label(&format!("Insert {}", letter as char)).count(), 2);
        assert_eq!(harness.query_all_by_label(&format!("Send {}", letter as char)).count(), 2);
    }
    assert!(harness.state().ui.mix_views.iter().any(|view| view == "instruments"));
    assert!(harness.query_all_by_label("Audio 1 Volume").count() > 0);
    assert!(harness.query_all_by_label("Audio 1 Pan").count() > 0);
}

#[test]
fn explicit_effect_drops_target_the_chosen_track_and_slot_and_undo() {
    let mut harness = harness();
    for name in ["MIDI 1", "Audio 1"] {
        let track = harness.state().engine.session().track_by_name(name).unwrap().id;
        soundcraft_ui_egui::devices::insert_effect_at(harness.state_mut(), track, 0, &BrowserItem::Plugin("compressor".into())).unwrap();
        soundcraft_ui_egui::devices::insert_effect_at(harness.state_mut(), track, 4, &BrowserItem::Plugin("eq_7band".into())).unwrap();
        let inserts = &harness.state().engine.session().track(track).unwrap().mixer.inserts;
        assert_eq!(inserts[0].as_ref().unwrap().plugin, "compressor");
        assert_eq!(inserts[4].as_ref().unwrap().plugin, "eq_7band");
        assert!(inserts[1..4].iter().all(Option::is_none));
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert!(harness.state().engine.session().track(track).unwrap().mixer.inserts[4].is_none());
        assert!(harness.state().engine.session().track(track).unwrap().mixer.inserts[0].is_some());
    }
}

#[test]
fn effect_slots_reject_instruments_samples_and_invalid_slots_without_replacing_audio() {
    let mut harness = harness();
    let track = harness.state().engine.session().track_by_name("Audio 1").unwrap().id;
    let before = harness.state().engine.session().clone();
    for (slot, item) in [
        (0, BrowserItem::Plugin("drum_rack".into())),
        (0, BrowserItem::Sample("missing.wav".into())),
        (10, BrowserItem::Plugin("compressor".into())),
    ] {
        assert!(soundcraft_ui_egui::devices::insert_effect_at(harness.state_mut(), track, slot, &item).is_err());
        assert_eq!(harness.state().engine.session(), &before);
    }
}

#[test]
fn old_preferences_expose_inserts_and_sends_once_and_preserve_later_view_choices() {
    let mut harness = harness();
    let mut saved = serde_json::to_value(&harness.state().ui).unwrap();
    saved.as_object_mut().unwrap().remove("insert_views_revision");
    harness.state_mut().ui = serde_json::from_value(saved).unwrap();
    for section in ["inserts_ae", "sends_ae"] {
        harness.state_mut().engine.session_mut().edit.set_flag(&format!("edit_view.{section}"), false);
    }
    harness.step();
    for section in ["inserts_ae", "sends_ae"] {
        assert!(harness.state().engine.session().edit.flag(&format!("edit_view.{section}")));
    }
    harness.state_mut().run("view.edit_inserts_ae", json!({})).unwrap();
    harness.step();
    assert!(!harness.state().engine.session().edit.flag("edit_view.inserts_ae"));
    let saved = serde_json::to_value(&harness.state().ui).unwrap();
    let restored: soundcraft_ui_egui::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.insert_views_revision, 1);
}

#[test]
fn edit_io_readouts_open_the_recording_tracks_volume_and_pan_controls() {
    let mut harness = harness();
    harness.get_by_label("Audio 1 Volume").click();
    harness.run_steps(3);
    assert!(harness.query_all_by_label_contains("Audio 1 · Volume").count() > 0);
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    harness.get_by_label("Audio 1 Pan").click();
    harness.run_steps(3);
    assert!(harness.query_all_by_label_contains("Audio 1 · Pan").count() > 0);
}
