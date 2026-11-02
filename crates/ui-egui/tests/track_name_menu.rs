//! Right-clicking a track's name opens the track menu, in the Edit and in the Mix window.

use egui::{Event, Modifiers, PointerButton, Pos2, Shape, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soundcraft_ui_egui::{MainWindow, Services, SoundApp};

fn right_click(h: &mut Harness<SoundApp>, at: Pos2) {
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
}

#[test]
fn right_click_on_a_track_name_opens_the_track_menu() {
    let mut engine = soundcraft_engine::Engine::default();
    engine.execute("track.new", &serde_json::json!({"name": "Voice"})).unwrap();
    let app = SoundApp::new(engine, None, Services::default());
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    let row = h.state().edit_layout.rows[0].1;
    right_click(&mut h, pos2(row[0] + 60.0, row[1] + 10.0));
    assert!(h.query_by_label("Make Inactive").is_some(), "the Edit window's name plate");

    h.key_press(egui::Key::Escape);
    h.state_mut().ui.window = MainWindow::Mix;
    h.run_steps(4);
    assert!(h.query_by_label("Make Inactive").is_none());
    let name = h.output().shapes.iter().filter_map(|shape| match &shape.shape {
        Shape::Text(text) if text.galley.text() == "Voice" => Some(text.visual_bounding_rect().center()),
        _ => None,
    }).max_by(|a, b| a.x.total_cmp(&b.x)).expect("Mix strip track name");
    right_click(&mut h, name);
    assert!(h.query_by_label("Make Inactive").is_some(), "the Mix strip's name plate");
}
