use crate::SoundApp;
use egui::{Event, Key, Modifiers};
use serde_json::json;

#[derive(Default)]
pub struct KeypadState {
    events: Vec<(Key, Modifiers, bool)>,
    recall: Option<u32>,
}

impl SoundApp {
    pub fn native_keypad_input(&mut self, context: &egui::Context, input: &mut egui::RawInput) {
        if !input.focused {
            self.keypad.recall = None;
            self.keypad.events.clear();
        }
        if input.events.iter().any(|event| matches!(event, Event::Key { key, pressed:true, .. } if *key != Key::F35)) {
            self.keypad.recall = None;
        }
        let text = context.egui_wants_keyboard_input() || self.dialogs.open.is_some();
        input.events.retain_mut(|event| {
            if let Event::Key { key: Key::F35, physical_key: Some(original), pressed, repeat, modifiers } = event {
                if text {
                    *event = Event::Key { key: *original, physical_key: Some(*original), pressed: *pressed, repeat: *repeat, modifiers: *modifiers };
                    return true;
                }
                if *pressed {
                    self.keypad.events.push((*original, *modifiers, *repeat));
                }
                return false;
            }
            true
        });
    }
}

pub fn handle(app: &mut SoundApp, context: &egui::Context) {
    let events = std::mem::take(&mut app.keypad.events);
    if app.dialogs.open.is_some() || context.egui_wants_keyboard_input() {
        app.keypad.recall = None;
        return;
    }
    for (key, modifiers, repeat) in events {
        if repeat && !matches!(key, Key::Plus | Key::Minus) {
            continue;
        }
        let digit = match key {
            Key::Num0 => Some(0), Key::Num1 => Some(1), Key::Num2 => Some(2), Key::Num3 => Some(3), Key::Num4 => Some(4),
            Key::Num5 => Some(5), Key::Num6 => Some(6), Key::Num7 => Some(7), Key::Num8 => Some(8), Key::Num9 => Some(9), _ => None,
        };
        let plain = !modifiers.command && !modifiers.ctrl && !modifiers.alt && !modifiers.shift;
        if plain && key == Key::Period {
            if let Some(number) = app.keypad.recall.take() {
                if number > 0 {
                    let _ = app.run("markers.recall", json!({"number": number}));
                }
            } else {
                app.keypad.recall = Some(0);
            }
            continue;
        }
        if plain && let (Some(number), Some(digit)) = (app.keypad.recall, digit) {
            app.keypad.recall = number.checked_mul(10).and_then(|number| number.checked_add(digit)).filter(|number| *number <= 99999);
            continue;
        }
        app.keypad.recall = None;
        if modifiers.command && !modifiers.ctrl && !modifiers.alt && !modifiers.shift {
            let command = match digit {
                Some(1) => Some("window.transport"), Some(2) => Some("setup.session"), Some(3) => Some("window.big_counter"),
                Some(4) => Some("window.automation"), Some(5) => Some("window.memory_locations"),
                Some(7) => Some("window.video_universe"), Some(8) => Some("window.beat_detective"), Some(9) => Some("window.video"), _ => None,
            };
            if let Some(command) = command {
                if !crate::menus::invoke_shortcut_dialog(app, command) {
                    let _ = app.run(command, json!({}));
                }
                continue;
            }
        }
        if modifiers.alt && !modifiers.command && !modifiers.ctrl && !modifiers.shift {
            let command = match digit {
                Some(1) => Some("window.time_ops"), Some(2) => Some("window.tempo_ops"), Some(3) => Some("window.midi_ops"),
                Some(4) => Some("window.rtp"), _ => None,
            };
            if let Some(command) = command {
                let _ = app.run(command, json!({}));
                continue;
            }
        }
        if key == Key::Plus || key == Key::Minus {
            let direction = if key == Key::Plus { 1 } else { -1 };
            if modifiers.alt && modifiers.ctrl && !modifiers.command && !modifiers.shift {
                let _ = app.run("view.grid", json!({"step": direction}));
            } else if modifiers.alt && modifiers.command && !modifiers.ctrl && !modifiers.shift {
                let _ = app.run("view.nudge", json!({"step": direction}));
            } else if plain {
                let _ = app.run("edit.nudge", json!({"direction": direction}));
            }
            continue;
        }
        if plain {
            if key == Key::Enter {
                crate::menus::invoke_shortcut_dialog(app, "markers.add");
                continue;
            }
            let command = match digit {
                Some(0) => Some("transport.toggle"), Some(1) => Some("transport.rewind"), Some(2) => Some("transport.fast_forward"),
                Some(3) => Some("transport.record"), Some(4) => Some("options.loop_playback"), Some(5) => Some("options.loop_record"),
                Some(6) => Some("options.quickpunch"), Some(7) => Some("options.click"), Some(8) => Some("options.countoff"),
                Some(9) => Some("options.midi_merge"), _ => None,
            };
            if let Some(command) = command {
                let _ = app.run(command, json!({}));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    fn app() -> SoundApp {
        SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default())
    }

    fn tagged(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key: Key::F35, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn keypad_window_shortcuts_leave_row_tool_events_intact() {
        let mut app = app();
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events = vec![tagged(Key::Num1, Modifiers::COMMAND),
            Event::Key { key: Key::Num1, physical_key: Some(Key::Num1), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }];
        let before = app.ui.show_transport;
        let tool = app.engine.session().edit.tool;
        app.native_keypad_input(&context, &mut input);
        assert_eq!(input.events.len(), 1);
        assert!(matches!(input.events.first(), Some(Event::Key { key: Key::Num1, .. })));
        handle(&mut app, &context);
        assert_ne!(app.ui.show_transport, before);
        assert_eq!(app.engine.session().edit.tool, tool);
    }

    #[test]
    fn keypad_enter_prompts_without_creating_a_marker_and_restores_enter_in_the_dialog() {
        let mut app = app();
        let context = egui::Context::default();
        app.keypad.events.push((Key::Enter, Modifiers::NONE, false));
        handle(&mut app, &context);
        assert_eq!(app.dialogs.open_name(), Some("new_memory_location"));
        assert!(app.engine.session().markers.is_empty());
        let mut input = egui::RawInput::default();
        input.events.push(tagged(Key::Enter, Modifiers::NONE));
        app.native_keypad_input(&context, &mut input);
        assert!(matches!(input.events.first(), Some(Event::Key { key: Key::Enter, .. })));
        assert!(app.keypad.events.is_empty());
    }

    #[test]
    fn period_number_period_recalls_locations_without_toggling_transport() {
        let mut app = app();
        let context = egui::Context::default();
        app.run("markers.add", json!({"name":"Verse", "at":48000})).unwrap();
        app.run("edit.select", json!({"start":0,"end":0})).unwrap();
        for key in [Key::Period, Key::Num1, Key::Period] {
            app.keypad.events.push((key, Modifiers::NONE, false));
        }
        handle(&mut app, &context);
        assert_eq!(app.engine.session().edit.selection.start, 48000);
        assert!(!app.engine.transport.playing);
    }
    #[test]
    fn ordinary_keys_cancel_an_incomplete_memory_location_number() {
        let mut app=app();
        let context=egui::Context::default();
        app.keypad.recall=Some(12);
        let mut input=egui::RawInput::default();
        input.events.push(Event::Key { key:Key::Escape, physical_key:Some(Key::Escape), pressed:true, repeat:false, modifiers:Modifiers::NONE });
        app.native_keypad_input(&context,&mut input);
        assert_eq!(app.keypad.recall,None);
        assert_eq!(input.events.len(),1);
    }

}
