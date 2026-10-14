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
