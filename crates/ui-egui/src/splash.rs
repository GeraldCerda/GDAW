use egui::{Align2, Color32, FontId, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

pub struct Splash {
    pub heading: String,
    pub detail: String,
    pub painted: bool,
    texture: Option<egui::TextureHandle>,
    attempted: bool,
}

impl Default for Splash {
    fn default() -> Self {
        Self { heading: "Starting GDAW".into(), detail: String::new(), painted: false, texture: None, attempted: false }
    }
}

impl Splash {
    pub fn phase(&mut self, heading: impl Into<String>, detail: impl Into<String>) {
        self.heading = heading.into();
        self.detail = detail.into();
        self.painted = false;
    }

    pub fn show(&mut self, ui: &mut Ui, scanning: bool) {
        if !self.attempted {
            self.attempted = true;
            match image::load_from_memory(include_bytes!("../../../Neon Chrome G in Futuristic Studio.png")) {
                Ok(image) => {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    self.texture = Some(ui.ctx().load_texture("gdaw_startup_art", pixels, egui::TextureOptions::LINEAR));
                }
                Err(error) => log::warn!("startup artwork: {error}"),
            }
        }
        let canvas = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(canvas, 0.0, Color32::from_rgb(7, 6, 12));
        let aspect = self.texture.as_ref().map_or(16.0 / 9.0, |texture| {
            let size = texture.size_vec2();
            size.x / size.y.max(1.0)
        });
        let width = (canvas.width() - 48.0).min(1160.0).min((canvas.height() - 210.0).max(1.0) * aspect).max(1.0);
        let size = vec2(width, width / aspect);
        let top = canvas.min.y + ((canvas.height() - size.y - 170.0) * 0.5).max(16.0);
        let artwork = Rect::from_min_size(pos2(canvas.center().x - size.x * 0.5, top), size);
        if let Some(texture) = &self.texture {
            painter.image(texture.id(), artwork, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.text(artwork.center(), Align2::CENTER_CENTER, "G", FontId::proportional(110.0), Color32::from_rgb(174, 142, 216));
        }
        let panel_width = width.min(760.0).max(1.0);
        let x = canvas.center().x - panel_width * 0.5;
        let y = artwork.max.y + 12.0;
        let progress = crate::plugin_scan::progress();
        let heading = if scanning { "Scanning installed plugins" } else { self.heading.as_str() };
        let fraction = if scanning { progress.fraction() } else if heading == "Ready" { Some(1.0) } else { None };
        ui.put(Rect::from_min_size(pos2(x, y), vec2(panel_width, 24.0)), egui::Label::new(egui::RichText::new(heading).size(17.0).color(Color32::from_rgb(224, 222, 230))));
        let bar = Rect::from_min_size(pos2(x, y + 32.0), vec2(panel_width, 7.0));
        painter.rect_filled(bar, 3.0, Color32::from_rgb(44, 39, 53));
        if let Some(fraction) = fraction {
            painter.rect_filled(bar.with_max_x(bar.min.x + bar.width() * fraction.clamp(0.0, 1.0)), 3.0, Color32::from_rgb(168, 133, 215));
        } else {
            let phase = ui.input(|input| (input.time * 0.5).fract()) as f32;
            let segment = bar.width() * 0.2;
            let start = bar.min.x + (bar.width() + segment) * phase - segment;
            painter.with_clip_rect(bar).rect_filled(Rect::from_min_size(pos2(start, bar.min.y), vec2(segment, bar.height())), 3.0, Color32::from_rgb(168, 133, 215));
        }
        let dim = Color32::from_rgb(176, 172, 187);
        if scanning {
            for (index, (key, label)) in [("--gdaw-scan-au", "Audio Units"), ("--gdaw-scan-vst3", "VST3"), ("--gdaw-scan-clap", "CLAP")].into_iter().enumerate() {
                let state = progress.formats.get(key);
                let current = progress.current.get(key);
                let summary = if state.is_some_and(|state| state.finished) { "Complete".into() }
                    else if let Some(path) = current { std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned() }
                    else if state.is_some_and(|state| state.total.is_some()) { "Checking plugin metadata".into() }
                    else { "Locating installed plugins".into() };
                let counts = state.and_then(|state| state.total.map(|total| format!("{} / {total}", state.completed))).unwrap_or_else(|| "…".into());
                let text = format!("{label}   {counts}   {summary}");
                ui.put(Rect::from_min_size(pos2(x, y + 46.0 + index as f32 * 21.0), vec2(panel_width, 20.0)), egui::Label::new(egui::RichText::new(text).size(12.0).color(dim)).truncate()).on_hover_text(current.cloned().unwrap_or_default());
            }
            let detail = format!("{} checked · {} cached · {} skipped", progress.completed, progress.cached, progress.failures.len());
            painter.text(pos2(canvas.center().x, y + 117.0), Align2::CENTER_TOP, detail, FontId::proportional(11.0), dim);
        } else {
            ui.put(Rect::from_min_size(pos2(x, y + 48.0), vec2(panel_width, 42.0)), egui::Label::new(egui::RichText::new(&self.detail).size(12.0).color(dim)).truncate());
        }
        painter.rect_stroke(artwork, 0.0, Stroke::new(1.0, Color32::from_rgb(28, 24, 37)), StrokeKind::Inside);
        self.painted = true;
        ui.ctx().request_repaint_after_secs(0.033);
    }
}
