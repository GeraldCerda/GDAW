use crate::{Engine, EngineError, Result};
use serde_json::{Value, json};
use soundcraft_audio_io::{BitDepth, EncodeOptions, FileFormat};
use soundcraft_model::{Route, Session, TrackKind};
use soundcraft_time::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

struct Staging { path: PathBuf, published: bool }

impl Drop for Staging {
    fn drop(&mut self) { if !self.published { let _ = std::fs::remove_dir_all(&self.path); } }
}

pub struct Exchange {
    source: Arc<Session>,
    staging: Staging,
    destination: PathBuf,
    range: Range,
    tasks: Vec<(Option<soundcraft_model::TrackId>, String)>,
    index: usize,
    current: Option<crate::io::Bounce>,
    rendered: Vec<Value>,
}

impl Exchange {
    pub fn new(engine: &Engine, params: &Value) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let fail = |message: &str| EngineError::BadParams("file.export_exchange".into(), message.into());
        if engine.transport.recording { return Err(fail("Stop recording before exporting the session")); }
        let path = params.get("dir").and_then(Value::as_str).filter(|path| !path.is_empty()).ok_or_else(|| fail("Choose a new export folder"))?;
        let destination = PathBuf::from(path);
        crate::file_access::check_write(&destination).map_err(EngineError::Io)?;
        if destination.exists() { return Err(fail("The export folder already exists. Choose a new folder name so no existing files are replaced.")); }
        let source = engine.session_arc();
        let range = if params.get("start").is_some() || params.get("end").is_some() { crate::cmd::range_param(engine, "file.export_exchange", params)? }
            else { Range::new(0, source.content_end().saturating_add(source.sample_rate.samples(2.0)).max(1)) };
        if range.is_empty() || range.start < 0 || range.len() > source.sample_rate.samples(14_400.0) { return Err(fail("Choose a nonempty range between zero and four hours")); }
        let mut tasks = vec![(None, "Reference Mix.wav".to_string())];
        for (index, track) in source.tracks.iter().filter(|track| !track.inactive && matches!(track.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi)).enumerate() {
            tasks.push((Some(track.id), format!("{:03}-{}-{}.wav", index + 1, track.id.0, crate::io::sanitize_name(&track.name))));
        }
        if tasks.len() == 1 { return Err(fail("There are no active audio or instrument tracks to export")); }
        let parent = destination.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        crate::io::make_dir(parent)?;
        let staging = (0..128).find_map(|_| {
            let path = parent.join(format!(".gdaw-export-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            if crate::file_access::check_write(&path).is_err() { return None; }
            std::fs::create_dir(&path).ok().map(|()| Staging { path, published: false })
        }).ok_or_else(|| fail("Cannot create a temporary export folder"))?;
        let midi = crate::io::export_midi(engine, None)?;
        crate::io::write_path(staging.path.join("Song.mid"), midi)?;
        crate::io::write_path(staging.path.join("Session Info.txt"), crate::inspect::session_text(engine))?;
        crate::io::write_path(staging.path.join("Session Document.json"), source.to_json().map_err(|error| EngineError::Io(error.to_string()))?)?;
        Ok(Self { source, staging, destination, range, tasks, index: 0, current: None, rendered: Vec::new() })
    }

    pub fn path(&self) -> &Path { &self.destination }
    pub fn progress(&self) -> f32 { (self.index as f32 + self.current.as_ref().map_or(0.0, crate::io::Bounce::progress)) / self.tasks.len().max(1) as f32 }

    fn begin_track(&mut self) -> Result<()> {
        let (track, name) = &self.tasks[self.index];
        let session = match track {
            None => Arc::clone(&self.source),
            Some(selected) => {
                let mut session = (*self.source).clone();
                for track in &mut session.tracks {
                    track.mixer.solo = false;
                    if track.id == *selected {
                        track.mixer.mute = false;
                        track.mixer.output = Route::Main;
                        track.mixer.sends.iter_mut().for_each(|send| *send = None);
                    } else if !matches!(track.mixer.output, Route::Bus(_)) { track.mixer.output = Route::None; }
                    if track.kind == TrackKind::Master && !matches!(track.mixer.output, Route::Bus(_)) { track.inactive = true; }
                    for send in track.mixer.sends.iter_mut().flatten() { if !matches!(send.target, Route::Bus(_)) { send.mute = true; } }
                }
                Arc::new(session)
            }
        };
        let options = EncodeOptions { format: FileFormat::Wav, bit_depth: BitDepth::Int24, dither: true, bwf: None };
        self.current = Some(crate::io::Bounce::new(session, &self.staging.path.join(name), self.range, &options, None, false, false)?);
        Ok(())
    }

    pub fn step(&mut self, mut more: impl FnMut() -> bool) -> Result<Option<Value>> {
        loop {
            if self.current.is_none() { self.begin_track()?; }
            let done = match &mut self.current { Some(job) => job.step(&mut more)?, None => None };
            let Some(done) = done else { return Ok(None) };
            let (track, file) = &self.tasks[self.index];
            self.rendered.push(json!({"track": track.map(|track| track.0), "file": file, "peak_dbfs": if done.peak_db.is_finite() { Some(done.peak_db) } else { None }, "frames": self.range.len(), "role": if track.is_some() { "post-track processing, without master or send returns" } else { "reference mix" }}));
            self.current = None;
            self.index += 1;
            if self.index == self.tasks.len() {
                let manifest = json!({"format": "gdaw-exchange", "version": 1, "session": self.source.name, "sample_rate": self.source.sample_rate.hz(), "bit_depth": 24, "channels": soundcraft_mix::main_channels(&self.source), "audio_origin_samples": self.range.start, "audio_end_samples": self.range.end, "midi_origin_samples": 0, "midi": "Song.mid", "document": "Session Document.json", "files": self.rendered, "notes": ["WAVs have equal lengths and the same start. Import them at audio_origin_samples.", "MIDI keeps the original song timeline and tempo map.", "Rendered track files preserve track processing; bus/send returns and master processing are present in the reference mix.", "The JSON document is metadata; use a .gdawz project to transfer the editable native session and media."]});
                let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| EngineError::Io(error.to_string()))?;
                crate::io::write_path(self.staging.path.join("GDAW Exchange.json"), bytes)?;
                if self.destination.exists() { return Err(EngineError::Io("The destination appeared during export. Choose another folder name.".into())); }
                std::fs::rename(&self.staging.path, &self.destination).map_err(|error| EngineError::Io(error.to_string()))?;
                self.staging.published = true;
                return Ok(Some(json!({"directory": self.destination, "files": self.rendered.len(), "format": "WAV 24-bit + MIDI + JSON", "start": self.range.start, "end": self.range.end})));
            }
            if !more() { return Ok(None); }
        }
    }

    pub fn run(mut self) -> Result<Value> {
        loop { if let Some(result) = self.step(|| true)? { return Ok(result); } }
    }
}

impl Drop for Exchange {
    fn drop(&mut self) { self.current = None; }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_an_export_removes_its_staging_files_without_publishing() {
        let root = std::env::temp_dir().join(format!("gdaw-cancel-export-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let destination = root.join("Export");
        let mut engine = Engine::default();
        engine.session_mut().add_track(TrackKind::Audio, soundcraft_model::ChannelFormat::Stereo, None);
        let mut job = Exchange::new(&engine, &json!({"dir": destination, "start": 0, "end": 4096})).unwrap();
        let staging = job.staging.path.clone();
        assert!(job.step(|| false).unwrap().is_none());
        drop(job);
        assert!(!staging.exists());
        assert!(!destination.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
