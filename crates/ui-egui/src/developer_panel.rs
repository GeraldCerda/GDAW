use crate::SoundApp;

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_developer_api { return; }
    let mut open = true;
    egui::Window::new("Build with GDAW").open(&mut open).default_width(480.0).show(ctx, |ui| {
        ui.heading("Your tools, connected to your session");
        ui.label("Scripts, controllers and assistants can use the same editing commands as GDAW's interface.");
        ui.separator();
        ui.strong("Local API · version 1");
        ui.label("Quit GDAW, then launch it with --control 7801 to enable the local command connection. It stays off unless you enable it.");
        if ui.button("Copy launch command").clicked() {
            if let Ok(path) = std::env::current_exe() { ctx.copy_text(format!("'{}' --control 7801", path.to_string_lossy().replace('\'', "'\\''"))); }
        }
        ui.hyperlink_to("Developer guide and Python client", "https://github.com/GeraldCerda/GDAW/blob/main/docs/GDAW_DEVELOPER_API.md");
        ui.label("Discover commands with engine.commands. Inspect the session, search samples, move devices, edit automation and request exports. Desktop control uses localhost; CLI and MCP access are also available.");
        ui.separator();
        ui.strong("Project and exchange files");
        ui.label(".gdaw keeps the editable native session with its media folder. .gdawz packages an audio project. WAV tracks and MIDI export brings rendered material into another DAW.");
        ui.small("Native Pro Tools/Ableton session files and AAF/OMF interchange are not implemented.");
    });
    app.ui.show_developer_api = open;
}
