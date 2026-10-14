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
