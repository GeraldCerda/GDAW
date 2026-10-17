//! Realtime playback through the system audio device (cpal: CoreAudio, WASAPI, ALSA/Pulse/
//! PipeWire via ALSA, JACK, WebAudio).
//!
//! The UI thread owns a [`Player`]; the audio callback owns the [`MixEngine`] and a snapshot of
//! the session (`Arc<Session>`) that the UI swaps whenever the document changes. Commands go to
//! the callback through a channel; position and meters come back through atomics and a
//! `try_lock`ed snapshot, so the callback never blocks. When no device is available the player
//! falls back to a silent clock thread so the transport, meters and automation still run.
//!
//! Surround: the device is opened with as many output channels as it offers, up to the width of
//! the session's main format. Main channels (SMPTE/WAV order) go to device channels in order; when
//! the device has fewer channels than the main mix, the mix is folded down to stereo (ITU-R
//! BS.775) on the first two device channels.
//!
//! Plugins and threads: hosted third-party plugins (CLAP/VST3) are created, given their stored
//! state and prepared on the thread that calls [`Player::update_session`] (the UI thread), and
//! travel to the audio thread together with the session snapshot that needs them; the mix
//! engine adopts them (see `soundcraft_mix`'s crate docs). Instances the engine retires, and the
//! session snapshots the callback replaces, come back on return channels and are dropped on the UI
//! thread ([`Player::idle`]). Plugin editors are driven, and plugin states for saving read
//! ([`Player::capture_states`]), on the UI thread
//! through handles taken from each instance before it leaves ([`Player::open_editor`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod devices;
pub mod record;
mod mailbox;

pub use devices::AudioConfig;

use soundcraft_dsp::{Plugin, PluginEditor, PluginStateReader};
use soundcraft_mix::{MAX_CHANNELS, MixEngine, PreparedInstance, StripMeter, itu_stereo_fold};
use soundcraft_model::{Session, TrackId};
use soundcraft_time::{Range, Samples};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use mailbox::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};

const BLOCK: usize = 512;

/// Room for replaced snapshots on their way back; `update_session` empties it before each send.
const OLD_SESSIONS: usize = 16;

/// The realtime audio thread marker lives in the mix engine, which carries it into the strips it
/// renders on worker threads; the cpal callbacks here set it.
pub use soundcraft_mix::{mark_audio_thread, on_audio_thread};

/// Meter values for the UI.
#[derive(Debug, Clone, Default)]
pub struct MeterSnapshot {
    pub tracks: HashMap<TrackId, StripMeter>,
    pub main: StripMeter,
}

/// Plugin states read from the live instances: `(track, slot or INSTRUMENT_SLOT, state)`.
pub type PluginStates = Vec<(TrackId, usize, Vec<u8>)>;

/// A parameter edit made in a plugin's own editor: `(track, slot, param id, value)`.
pub type EditorEdit = (TrackId, usize, String, f32);

enum Cmd {
    Ready(Box<PreparedPlayback>),
    ReadyInput(Box<PreparedInput>),
    Reclaim(Sender<Box<PreparedPlayback>>, Sender<Box<PreparedInput>>, Sender<Arc<soundcraft_model::SourceAudio>>),
    Preview(Option<Arc<soundcraft_model::SourceAudio>>),
    PreviewGain(f32),
    /// A new snapshot plus the third-party instances it needs (created on the UI thread).
    #[cfg(test)]
    Session(Arc<Session>, Vec<PreparedInstance>),
    /// Where the callback sends the snapshots it replaces (freeing one takes thousands of frees).
    OldSessions(SyncSender<Arc<Session>>),
    #[cfg(test)]
    Input(Arc<record::InputRing>),
    Recording(bool),
    Audition(TrackId, u8),
    Play {
        from: Samples,
        end: Option<Samples>,
        looped: Option<Range>,
    },
    Stop,
    Locate(Samples),
}

struct PreparedPlayback {
    session: Arc<Session>,
    mix: MixEngine,
    pending: Vec<Vec<f32>>,
    fresh: Vec<(TrackId, usize)>,
}

struct PreparedInput {
    ring: Option<Arc<record::InputRing>>,
    buffer: Vec<Vec<f32>>,
}

impl PreparedPlayback {
    fn new(session: Arc<Session>, instances: Vec<PreparedInstance>) -> Self {
        let fresh = instances.iter().map(|instance| (instance.track, instance.slot)).collect();
        let mut mix = new_engine(session.sample_rate.as_f64() as f32);
        mix.adopt(instances);
        mix.ensure_synced(&session);
        let pending = vec![vec![0.0; BLOCK]; soundcraft_mix::device_channels(&session)];
        Self { session, mix, pending, fresh }
    }
}

#[derive(Default)]
struct RecordClock {
    nanos: AtomicU64,
    position: AtomicI64,
    rate: AtomicU32,
}

struct Shared {
    record_clock: RecordClock,
    position: AtomicI64,
    playing: AtomicBool,
    auditioning: AtomicBool,
    /// Speed × 1000.
    speed: AtomicU32,
    meters: MeterMailbox,
    session_pending: AtomicBool,
}

const METER_TRACKS: usize = 4096;

#[derive(Default)]
struct AtomicMeter {
    id: AtomicU64,
    channels: AtomicUsize,
    peaks: [AtomicU32; MAX_CHANNELS],
    stereo: [AtomicU32; 2],
    reduction: AtomicU32,
    failures: AtomicU32,
    last_failure: AtomicUsize,
}

impl AtomicMeter {
    fn publish(&self, id: TrackId, meter: Option<&StripMeter>) {
        self.id.store(id.0, Ordering::Relaxed);
        self.channels.store(meter.map_or(0, |meter| meter.peaks.len().min(MAX_CHANNELS)), Ordering::Relaxed);
        for (i, value) in self.peaks.iter().enumerate() {
            value.store(meter.and_then(|meter| meter.peaks.get(i)).copied().unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        }
        for (i, value) in self.stereo.iter().enumerate() {
            value.store(meter.map_or(0.0, |meter| meter.peak[i]).to_bits(), Ordering::Relaxed);
        }
        self.reduction.store(meter.map_or(0.0, |meter| meter.gain_reduction_db).to_bits(), Ordering::Relaxed);
        self.failures.store(meter.map_or(0, |meter| meter.plugin_failures.count), Ordering::Relaxed);
        self.last_failure.store(meter.map_or(0, |meter| meter.plugin_failures.last), Ordering::Relaxed);
    }

    fn read(&self) -> StripMeter {
        StripMeter {
            peak: std::array::from_fn(|i| f32::from_bits(self.stereo[i].load(Ordering::Relaxed))),
            peaks: self.peaks.iter().take(self.channels.load(Ordering::Relaxed).min(MAX_CHANNELS)).map(|value| f32::from_bits(value.load(Ordering::Relaxed))).collect(),
            gain_reduction_db: f32::from_bits(self.reduction.load(Ordering::Relaxed)),
            plugin_failures: soundcraft_mix::PluginFailures {
                count: self.failures.load(Ordering::Relaxed),
                last: self.last_failure.load(Ordering::Relaxed),
            },
        }
    }
}

struct MeterMailbox {
    sequence: AtomicU64,
    count: AtomicUsize,
    tracks: Box<[AtomicMeter]>,
    main: AtomicMeter,
}

impl Default for MeterMailbox {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0), count: AtomicUsize::new(0),
            tracks: (0..METER_TRACKS).map(|_| AtomicMeter::default()).collect(), main: AtomicMeter::default(),
        }
    }
}

impl MeterMailbox {
    fn publish(&self, tracks: &HashMap<TrackId, StripMeter>, main: &StripMeter) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        for (slot, (id, meter)) in self.tracks.iter().zip(tracks) {
            slot.publish(*id, Some(meter));
        }
        self.count.store(tracks.len().min(self.tracks.len()), Ordering::Relaxed);
        self.main.publish(TrackId(0), Some(main));
        self.sequence.fetch_add(1, Ordering::Release);
    }

    fn clear(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.count.store(0, Ordering::Relaxed);
        self.main.publish(TrackId(0), None);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    fn try_snapshot(&self) -> Option<MeterSnapshot> {
        for _ in 0..3 {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let count = self.count.load(Ordering::Relaxed).min(self.tracks.len());
            let tracks = self.tracks.iter().take(count).map(|slot| (TrackId(slot.id.load(Ordering::Relaxed)), slot.read())).collect();
            let main = self.main.read();
            std::sync::atomic::fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == before {
                return Some(MeterSnapshot { tracks, main });
            }
        }
        None
    }

    fn snapshot(&self) -> MeterSnapshot {
        self.try_snapshot().unwrap_or_default()
    }
}

/// What stopped the audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTrouble {
    /// A block panicked; the output stays silent.
    Bug,
    /// The device went away.
    Gone,
    /// The device stopped asking for audio while playing.
    Stalled,
}

/// How the audio output is doing, for the UI.
#[derive(Default)]
struct Fault {
    /// A block panicked: output stays silent from then on.
    failed: AtomicBool,
    gone: AtomicBool,
    /// Blocks so far, to tell a device that stopped asking for audio.
    beats: std::sync::atomic::AtomicU64,
    /// The UI has been told.
    reported: AtomicBool,
}

impl Fault {
    /// What stopped the output, once.
    fn trouble(&self, stalled: bool) -> Option<OutputTrouble> {
        let trouble = if self.failed.load(Ordering::Relaxed) {
            OutputTrouble::Bug
        } else if self.gone.load(Ordering::Relaxed) {
            OutputTrouble::Gone
        } else if stalled {
            OutputTrouble::Stalled
        } else {
            return None;
        };
        (!self.reported.swap(true, Ordering::Relaxed)).then_some(trouble)
    }
}

/// How long a playing output may go without a block before it counts as stalled.
#[cfg(not(target_arch = "wasm32"))]
const STALL: std::time::Duration = std::time::Duration::from_secs(2);

/// True once the block count `beats` has stood still for `STALL` while playing; `seen` is the
/// count last seen and since when.
#[cfg(not(target_arch = "wasm32"))]
fn stalled(seen: &mut Option<(u64, std::time::Instant)>, beats: u64, playing: bool) -> bool {
    match *seen {
        Some((b, since)) if playing && b == beats => since.elapsed() >= STALL,
        _ => {
            *seen = Some((beats, std::time::Instant::now()));
            false
        }
    }
}

/// Fill one block with `fill`. If it panics, the block and every later one are silent and the
/// transport stops, rather than the panic taking the app down.
fn guarded_block(fault: &Fault, shared: &Shared, out: &mut [f32], fill: impl FnOnce(&mut [f32])) {
    fault.beats.fetch_add(1, Ordering::Relaxed);
    if fault.failed.load(Ordering::Relaxed) || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(out))).is_err() {
        fault.failed.store(true, Ordering::Relaxed);
        shared.playing.store(false, Ordering::Relaxed);
        out.fill(0.0);
    }
}

/// State owned by the audio callback.
struct AudioState {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    session: Arc<Session>,
    mix: MixEngine,
    pos: Samples,
    playing: bool,
    end: Option<Samples>,
    looped: Option<Range>,
    /// Rendered frames waiting to be consumed (planar, main channels) and the fractional read index.
    pending: Vec<Vec<f32>>,
    /// Stereo fold-down gains per main channel (used when the device has fewer channels).
    fold: [(f32, f32); MAX_CHANNELS],
    pending_len: usize,
    read: f64,
    device_rate: f64,
    input: Option<Arc<record::InputRing>>,
    recording: bool,
    audition_frames: usize,
    /// Retired plugin instances go back to the UI thread to be dropped there.
    garbage: Sender<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<SyncSender<Arc<Session>>>,
    retired: Option<Box<PreparedPlayback>>,
    retirement: Option<Sender<Box<PreparedPlayback>>>,
    old_input: Option<Box<PreparedInput>>,
    input_retirement: Option<Sender<Box<PreparedInput>>>,
    pending_garbage: Option<Vec<Box<dyn Plugin>>>,
    preview: Option<Arc<soundcraft_model::SourceAudio>>,
    old_preview: Option<Arc<soundcraft_model::SourceAudio>>,
    preview_retirement: Option<Sender<Arc<soundcraft_model::SourceAudio>>>,
    preview_position: f64,
    preview_gain: f32,
}

fn new_engine(sr: f32) -> MixEngine {
    let mut m = MixEngine::new(sr, BLOCK);
    m.set_external_instances(true);
    m.metronome = true;
    m
}

impl AudioState {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, session: Arc<Session>, device_rate: f64, garbage: Sender<Vec<Box<dyn Plugin>>>) -> Self {
        let sr = session.sample_rate.as_f64() as f32;
        let nch = soundcraft_mix::device_channels(&session);
        let mut mix = new_engine(sr);
        mix.ensure_synced(&session);
        AudioState {
            garbage,
            old_sessions: None,
            retired: None,
            retirement: None,
            old_input: None,
            input_retirement: None,
            pending_garbage: None,
            preview: None,
            old_preview: None,
            preview_retirement: None,
            preview_position: 0.0,
            preview_gain: 0.25,
            fold: fold_gains(&session),
            rx,
            shared,
            mix,
            session,
            pos: 0,
            playing: false,
            end: None,
            looped: None,
            pending: vec![vec![0.0; BLOCK]; nch],
            pending_len: 0,
            read: 0.0,
            device_rate,
            input: None,
            recording: false,
            audition_frames: 0,
        }
    }

    fn drain(&mut self) {
        #[cfg(test)]
        let mut changed = false;
        if let Some(old) = self.retired.take() {
            if let Some(sender) = &self.retirement {
                if let Err(error) = sender.send(old) { self.retired = Some(error.0); return; }
            } else { self.retired = Some(old); return; }
        }
        if let Some(old) = self.old_input.take() {
            if let Some(sender) = &self.input_retirement {
                if let Err(error) = sender.send(old) { self.old_input = Some(error.0); return; }
            } else { self.old_input = Some(old); return; }
        }
        if let Some(old) = self.old_preview.take() {
            if let Some(sender) = &self.preview_retirement {
                if let Err(error) = sender.send(old) { self.old_preview = Some(error.0); return; }
            } else { self.old_preview = Some(old); return; }
        }
        for _ in 0..64 {
            let Ok(c) = self.rx.try_recv() else { break };
            match c {
                Cmd::Reclaim(states, inputs, previews) => { self.retirement = Some(states); self.input_retirement = Some(inputs); self.preview_retirement = Some(previews); }
                Cmd::Preview(preview) => {
                    self.old_preview = std::mem::replace(&mut self.preview, preview);
                    self.preview_position = 0.0;
                    break;
                }
                Cmd::PreviewGain(gain) => self.preview_gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.25 },
                Cmd::ReadyInput(mut prepared) => {
                    std::mem::swap(&mut self.input, &mut prepared.ring);
                    std::mem::swap(&mut self.mix.input, &mut prepared.buffer);
                    self.old_input = Some(prepared);
                    break;
                }
                Cmd::Ready(mut ready) => {
                    ready.mix.inherit(&mut self.mix, &ready.session, &self.session, &ready.fresh);
                    let format_changed = self.pending.len() != ready.pending.len() || self.session.sample_rate != ready.session.sample_rate;
                    std::mem::swap(&mut self.mix, &mut ready.mix);
                    std::mem::swap(&mut self.session, &mut ready.session);
                    if format_changed {
                        std::mem::swap(&mut self.pending, &mut ready.pending);
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&self.session);
                    self.retired = Some(ready);
                    self.shared.session_pending.store(false, Ordering::Release);
                    break;
                }
                #[cfg(test)]
                Cmd::Input(r) => {
                    self.mix.input = vec![vec![0.0; BLOCK]; r.channels.load(Ordering::Relaxed).max(1)];
                    self.input = Some(r);
                }
                Cmd::Recording(r) => self.recording = r,
                Cmd::Audition(track, note) => {
                    self.mix.monitor_only = !self.playing;
                    let previous = if self.mix.audition_track == Some(track) { self.audition_frames } else { 0 };
                    let frames = self.mix.audition(&self.session, track, note);
                    let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
                    let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.1, 4.0);
                    self.audition_frames = ((frames as f64 / step).ceil().min(usize::MAX as f64) as usize).max(previous);
                    if !self.playing {
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                }
                #[cfg(test)]
                Cmd::Session(s, instances) => {
                    changed = true;
                    if (s.sample_rate.as_f64() - self.session.sample_rate.as_f64()).abs() > 0.5 {
                        let mut old = std::mem::replace(&mut self.mix, new_engine(s.sample_rate.as_f64() as f32));
                        let _ = self.garbage.send(old.drain_plugins());
                    }
                    self.mix.adopt(instances);
                    // The main format changed (a document edit, not steady state): resize.
                    let nch = soundcraft_mix::device_channels(&s);
                    if self.pending.len() != nch {
                        self.pending = vec![vec![0.0; BLOCK]; nch];
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&s);
                    let old = std::mem::replace(&mut self.session, s);
                    if let Some(tx) = &self.old_sessions {
                        let _ = tx.try_send(old);
                    }
                }
                Cmd::OldSessions(tx) => self.old_sessions = Some(tx),
                Cmd::Play { from, end, looped } => {
                    self.audition_frames = 0;
                    self.pos = from;
                    self.end = end;
                    self.looped = looped.filter(|r| r.len() > 64);
                    self.playing = true;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.mix.reset();
                }
                Cmd::Stop => {
                    self.audition_frames = 0;
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    self.mix.reset();
                }
                Cmd::Locate(at) => {
                    self.audition_frames = 0;
                    self.pos = at;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.mix.reset();
                }
            }
        }
        #[cfg(test)]
        if changed {
            // Place adopted instances now (not only once playback starts), so editors see them;
            // `sync` creates no third-party plugins here.
            self.mix.ensure_synced(&self.session);
        }
        self.send_garbage();
        self.shared.playing.store(self.playing, Ordering::Relaxed);
        self.shared.auditioning.store(self.audition_frames > 0, Ordering::Relaxed);
    }

    fn send_garbage(&mut self) {
        let g = self.pending_garbage.take().unwrap_or_else(|| self.mix.take_retired());
        if !g.is_empty() {
            if let Err(error) = self.garbage.send(g) { self.pending_garbage = Some(error.0); }
        }
    }

    /// Render the next block at the session rate into `pending`.
    fn render_next(&mut self, quantum: usize) {
        let mut n = quantum.clamp(1, BLOCK);
        if let Some(l) = self.looped
            && self.pos >= l.end
        {
            self.pos = l.start;
        }
        if let Some(l) = self.looped {
            n = n.min(usize::try_from((l.end - self.pos).max(1)).unwrap_or(BLOCK));
        } else if let Some(e) = self.end {
            if self.pos >= e {
                self.playing = false;
                self.end = None;
                self.shared.playing.store(false, Ordering::Relaxed);
                n = 0;
            } else {
                n = n.min(usize::try_from(e - self.pos).unwrap_or(BLOCK));
            }
        }
        if n == 0 {
            for c in &mut self.pending {
                c.iter_mut().for_each(|x| *x = 0.0);
            }
            self.pending_len = BLOCK;
            return;
        }
        if let Some(ring) = &self.input {
            ring.pop_into(&mut self.mix.input, n, self.session.sample_rate.hz());
        }
        self.mix.monitor_only = !self.playing;
        self.mix.recording = self.recording;
        self.mix.render_device(&self.session, self.pos, n, &mut self.pending);
        self.send_garbage();
        self.pending_len = n;
        self.shared.meters.publish(&self.mix.meters, &self.mix.main_meter);
        if self.playing {
            self.pos = self.pos.saturating_add(n as i64);
            self.shared.position.store(self.pos, Ordering::Relaxed);
        }
    }

    /// Fill an interleaved device buffer.
    fn fill_preview(&mut self, output: &mut [f32], channels: usize) {
        let Some(audio) = &self.preview else { return };
        let frames = audio.frames();
        if frames == 0 || audio.buffer.channels.is_empty() { return; }
        let rate = f64::from(audio.buffer.sample_rate.max(1));
        let step = rate / self.device_rate.max(1.0);
        let fade = (rate * 0.005).max(1.0);
        let first = self.session.outputs.first().map_or(0, |path| usize::from(path.first_channel));
        for frame in output.chunks_mut(channels.max(1)) {
            let index = self.preview_position.floor() as usize;
            if index >= frames { break; }
            let fraction = (self.preview_position - index as f64) as f32;
            let envelope = (self.preview_position / fade).min((frames.saturating_sub(index)) as f64 / fade).clamp(0.0, 1.0) as f32;
            for channel in 0..2 {
                let Some(destination) = frame.get_mut(first.saturating_add(channel)) else { continue };
                let Some(source) = audio.buffer.channels.get(channel.min(audio.buffer.channels.len() - 1)) else { continue };
                let a = source.get(index).copied().unwrap_or(0.0);
                let b = source.get(index.saturating_add(1)).copied().unwrap_or(a);
                let sample = (a + (b - a) * fraction) * envelope * self.preview_gain;
                if sample.is_finite() { *destination += sample; }
            }
            self.preview_position += step;
        }
    }

    fn fill(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let monitoring = self.input.is_some() && self.session.tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm || (matches!(t.kind, soundcraft_model::TrackKind::Aux | soundcraft_model::TrackKind::Folder) && t.mixer.input.hardware_input_start().is_some()));
        if !self.playing && !monitoring && self.audition_frames == 0 {
            self.mix.flush_parameters(&self.session);
            if let Some(input) = &self.input {
                input.discard();
            }
            out.iter_mut().for_each(|x| *x = 0.0);
            self.shared.meters.clear();
            return;
        }
        let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
        let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.1, 4.0);
        let callback_frames = out.len() / channels;
        let quantum = ((callback_frames as f64 * step).ceil() as usize).clamp(1, BLOCK);
        for frame in out.chunks_mut(channels) {
            while self.read >= self.pending_len as f64 {
                self.read -= self.pending_len as f64;
                if !self.playing && !monitoring && self.audition_frames == 0 {
                    break;
                }
                self.render_next(quantum);
                if self.pending_len == 0 {
                    break;
                }
            }
            let i0 = self.read.floor() as usize;
            let fr = (self.read - i0 as f64) as f32;
            let sounding = self.playing || monitoring || self.audition_frames > 0;
            let nch = self.pending.len();
            let sample = |c: usize| {
                let ch = self.pending.get(c);
                let a = ch.and_then(|v| v.get(i0)).copied().unwrap_or(0.0);
                let b = ch.and_then(|v| v.get(i0 + 1)).copied().unwrap_or(a);
                a + (b - a) * fr
            };
            if nch > 2 && channels < nch && self.session.outputs.len() <= 1 {
                // Fewer device channels than the main mix: ITU fold-down to stereo.
                let (mut l, mut r) = (0.0f32, 0.0f32);
                if sounding {
                    for (c, (gl, gr)) in self.fold.iter().enumerate().take(nch) {
                        let v = sample(c);
                        l += v * gl;
                        r += v * gr;
                    }
                }
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = match c {
                        0 => l.clamp(-1.0, 1.0),
                        1 => r.clamp(-1.0, 1.0),
                        _ => 0.0,
                    };
                }
            } else {
                for (c, o) in frame.iter_mut().enumerate() {
                    // Stereo mixes on a mono device play the left channel (as before).
                    *o = if c < nch.max(1) && sounding { sample(c).clamp(-1.0, 1.0) } else { 0.0 };
                }
            }
            self.read += step;
            if self.audition_frames > 0 {
                self.audition_frames -= 1;
                if self.audition_frames == 0 {
                    self.shared.auditioning.store(false, Ordering::Relaxed);
                    if self.playing {
                        self.mix.audition_track = None;
                    } else {
                        self.mix.end_audition();
                    }
                }
            }
        }
    }
}

/// UI-thread bookkeeping of the third-party instances the audio thread holds.
struct Side {
    /// What was last created for each slot: (plugin id, channels, created ok).
    shadow: HashMap<(TrackId, usize), (String, usize, bool)>,
    /// Session sample rate the instances were prepared for.
    rate: f32,
    /// Editor handles of the live instances.
    editors: HashMap<(TrackId, usize), Box<dyn PluginEditor>>,
    layouts: HashMap<(TrackId, usize), (bool, usize)>,
    load_failures: Vec<(String, String)>,
    /// State readers of the live instances.
    states: BTreeMap<(TrackId, usize), Box<dyn PluginStateReader>>,
    /// Instances that refused their stored state: saves keep that state until the plugin's editor
    /// is opened.
    refused: HashSet<(TrackId, usize)>,
    garbage: Receiver<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<Receiver<Arc<Session>>>,
    retired_states: Option<Receiver<Box<PreparedPlayback>>>,
    retired_inputs: Option<Receiver<Box<PreparedInput>>>,
    retired_previews: Option<Receiver<Arc<soundcraft_model::SourceAudio>>>,
    latest_session: Option<Arc<Session>>,
}

impl Side {
    fn new(rate: f32, garbage: Receiver<Vec<Box<dyn Plugin>>>) -> Side {
        Side { shadow: HashMap::new(), rate, editors: HashMap::new(), layouts: HashMap::new(), load_failures: Vec::new(), states: BTreeMap::new(), refused: HashSet::new(), garbage, old_sessions: None, retired_states: None, retired_inputs: None, retired_previews: None, latest_session: None }
    }

    /// Drops retired instances (closing their editors first, inside the plugin's drop) and
    /// replaced session snapshots.
    fn collect_garbage(&mut self) {
        if let Some(receiver) = &self.retired_states { receiver.try_iter().for_each(drop); }
        if let Some(receiver) = &self.retired_inputs { receiver.try_iter().for_each(drop); }
        if let Some(receiver) = &self.retired_previews { receiver.try_iter().for_each(drop); }
        while let Ok(g) = self.garbage.try_recv() {
            for p in g {
                log::debug!("plugin instance released: {}", p.info().name);
                drop(p);
            }
        }
        if let Some(rx) = &self.old_sessions {
            rx.try_iter().for_each(drop);
        }
    }

    fn return_old_sessions(&mut self, tx: &Sender<Cmd>) {
        let (states, receiver) = sync_channel(OLD_SESSIONS);
        self.retired_states = Some(receiver);
        let (inputs, receiver) = sync_channel(OLD_SESSIONS);
        self.retired_inputs = Some(receiver);
        let (previews, receiver) = sync_channel(OLD_SESSIONS);
        self.retired_previews = Some(receiver);
        let _ = tx.send(Cmd::Reclaim(states, inputs, previews));
        let (otx, orx) = sync_channel(OLD_SESSIONS);
        self.old_sessions = Some(orx);
        let _ = tx.send(Cmd::OldSessions(otx));
    }

    fn forget(&mut self, key: (TrackId, usize)) {
        self.layouts.remove(&key);
        self.shadow.remove(&key);
        self.states.remove(&key);
        self.refused.remove(&key);
        if let Some(mut e) = self.editors.remove(&key) {
            e.close();
        }
    }
}

/// The transport and audio device.
pub struct Player {
    shared: Arc<Shared>,
    fault: Arc<Fault>,
    /// The block count last seen while playing, and since when.
    #[cfg(not(target_arch = "wasm32"))]
    watch: Mutex<Option<(u64, std::time::Instant)>>,
    tx: Sender<Cmd>,
    side: Mutex<Side>,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub device_rate: u32,
    /// Output channels the device was opened with (0 for the silent clock).
    pub device_channels: u16,
    /// The device buffer in frames when one was chosen (`None`: the device default).
    pub device_buffer: Option<u32>,
    /// True when no audio device could be opened (silent clock).
    pub silent: bool,
}

impl Player {
    /// Open the default output device. Never fails: without a device it runs a silent clock.
    pub fn new(session: Arc<Session>) -> Player {
        Player::with_config(session, &AudioConfig::default())
    }

    /// Open the output device `cfg` chooses (system, device, rate, buffer size), falling back to the
    /// defaults for whatever is unavailable. Never fails: without a device it runs a silent clock.
    pub fn with_config(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let p = Player::open(Arc::clone(&session), cfg);
        p.side().return_old_sessions(&p.tx);
        // Hand the initial session's third-party plugins over.
        p.update_session(session);
        p
    }

    /// What stopped the audio output, once: a bug in the engine, or the device went away or
    /// stopped asking for audio while playing.
    pub fn trouble(&self) -> Option<OutputTrouble> {
        #[cfg(not(target_arch = "wasm32"))]
        let stalled = {
            let playing = self.shared.playing.load(Ordering::Relaxed) && !self.silent;
            stalled(&mut self.watch.lock().unwrap_or_else(PoisonError::into_inner), self.fault.beats.load(Ordering::Relaxed), playing)
        };
        #[cfg(target_arch = "wasm32")]
        let stalled = false;
        self.fault.trouble(stalled)
    }

    fn send(&self, command: Cmd) -> Result<(), std::sync::mpsc::SendError<Cmd>> {
        let result = self.tx.send(command);
        if result.is_err() { self.fault.failed.store(true, Ordering::Relaxed); }
        result
    }

    fn side(&self) -> std::sync::MutexGuard<'_, Side> {
        self.side.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let _ = record::clock_nanos();
        let want = session.sample_rate.hz();
        let want_ch = 64;
        let side_rate = session.sample_rate.as_f64() as f32;
        let new_side = |garbage: Receiver<Vec<Box<dyn Plugin>>>| Mutex::new(Side::new(side_rate, garbage));
        match open_device(want, want_ch, cfg) {
            Ok((device, config, format, name)) => {
                // The chosen buffer size first; if the device refuses it, its default.
                let mut attempts = vec![config.clone()];
                if config.buffer_size != cpal::BufferSize::Default {
                    attempts.push(cpal::StreamConfig { buffer_size: cpal::BufferSize::Default, ..config });
                }
                for config in attempts {
                    let rate = config.sample_rate.0;
                    let channels = usize::from(config.channels);
                    let (tx, rx) = channel();
                    let (gtx, grx) = channel();
                    let state = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&session), f64::from(rate), gtx);
                    let fault = Arc::new(Fault::default());
                    use cpal::traits::StreamTrait;
                    let built = match format {
                        cpal::SampleFormat::I32 => build_output::<i32>(&device, &config, state, channels, &fault, &shared),
                        cpal::SampleFormat::I16 => build_output::<i16>(&device, &config, state, channels, &fault, &shared),
                        _ => build_output::<f32>(&device, &config, state, channels, &fault, &shared),
                    };
                    match built {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("audio stream failed to start: {e}");
                            }
                            let buffer = match config.buffer_size {
                                cpal::BufferSize::Fixed(n) => Some(n),
                                cpal::BufferSize::Default => None,
                            };
                            log::info!(
                                "audio device: {name} @ {rate} Hz, {channels} ch, {format}, buffer {}",
                                buffer.map_or("default".to_string(), |n| n.to_string())
                            );
                            return Player {
                                shared,
                                fault,
                                #[cfg(not(target_arch = "wasm32"))]
                                watch: Mutex::new(None),
                                tx,
                                side: new_side(grx),
                                _stream: Some(stream),
                                device_name: name,
                                device_rate: rate,
                                device_channels: config.channels,
                                device_buffer: buffer,
                                silent: false,
                            };
                        }
                        Err(e) => log::warn!("cannot open audio stream ({:?}): {e}", config.buffer_size),
                    }
                }
                log::warn!("no usable audio stream; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock(shared, tx, rx, want, new_side(grx), gtx)
            }
            Err(e) => {
                log::warn!("no audio output: {e}; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock_with(shared, tx, rx, Arc::clone(&session), new_side(grx), gtx)
            }
        }
    }

    fn silent_clock(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        rate: u32,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let s = Arc::new(Session::new("Untitled", soundcraft_time::SampleRate::new(rate).unwrap_or_default()));
        Player::silent_clock_with(shared, tx, rx, s, side, gtx)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let rate = session.sample_rate.hz();
        let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
        let fault = Arc::new(Fault::default());
        let failed = Arc::clone(&fault);
        let spawn = std::thread::Builder::new().name("soundcraft-clock".into()).spawn(move || {
            let mut buf = vec![0.0f32; 4096];
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let now = std::time::Instant::now();
                let frames = ((now - last).as_secs_f64() * state.device_rate) as usize;
                last = now;
                let n = (frames * 2).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    let shared = Arc::clone(&state.shared);
                    guarded_block(&failed, &shared, b, |b| state.fill(b, 2));
                }
                if Arc::strong_count(&state.shared) <= 1 {
                    break;
                }
            }
        });
        if let Err(e) = spawn {
            log::warn!("clock thread failed: {e}");
        }
        Player {
            shared,
            fault,
            watch: Mutex::new(None),
            tx,
            side,
            _stream: None,
            device_name: "No audio device (silent)".into(),
            device_rate: rate,
            device_channels: 0,
            device_buffer: None,
            silent: true,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        _rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        _gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        Player {
            shared,
            fault: Arc::default(),
            tx,
            side,
            _stream: None,
            device_name: "No audio device".into(),
            device_rate: session.sample_rate.hz(),
            device_channels: 0,
            device_buffer: None,
            silent: true,
        }
    }

    /// Sends a new session snapshot to the audio thread, with freshly created (state-restored,
    /// prepared) instances of any third-party plugin it newly needs. Call on the UI thread.
    pub fn update_session(&self, s: Arc<Session>) {
        self.side().latest_session = Some(s);
        self.flush_session();
    }

    fn flush_session(&self) {
        let mut side = self.side();
        side.collect_garbage();
        if side.latest_session.is_none() || self.shared.session_pending.swap(true, Ordering::AcqRel) { return; }
        let Some(session) = side.latest_session.take() else { self.shared.session_pending.store(false, Ordering::Release); return };
        let instances = side.prepare(&session);
        drop(side);
        if self.send(Cmd::Ready(Box::new(PreparedPlayback::new(session, instances)))).is_err() {
            self.shared.session_pending.store(false, Ordering::Release);
        }
    }

    /// Reads the state of every live third-party plugin, for saving the session.
    pub fn capture_states(&self) -> PluginStates {
        self.side().capture_states()
    }

    /// Per-UI-frame housekeeping: drops retired instances and services plugin editors. Returns
    /// the parameter edits made in open editors since the last call.
    pub fn idle(&self) -> Vec<EditorEdit> {
        self.flush_session();
        let mut side = self.side();
        side.collect_garbage();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            for (id, v) in e.idle() {
                out.push((t, slot, id, v));
            }
        }
        out
    }

    pub fn editor_gestures(&self) -> Vec<(TrackId, usize, String, bool)> {
        let mut side = self.side();
        let mut out = Vec::new();
        for (&(track, slot), editor) in side.editors.iter_mut() {
            for (parameter, begin) in editor.take_gestures() { out.push((track, slot, parameter, begin)); }
        }
        out
    }

    /// True when the plugin in this slot has its own editor.
    pub fn has_editor(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.contains_key(&(track, slot))
    }

    pub fn preview_sample(&self, audio: Arc<soundcraft_model::SourceAudio>) -> Result<(), String> {
        if self.silent { return Err("Choose an audio output to preview samples".into()); }
        self.send(Cmd::Preview(Some(audio))).map_err(|_| "Sample preview is busy; try again".into())
    }

    pub fn stop_sample_preview(&self) { let _ = self.send(Cmd::Preview(None)); }

    pub fn sample_preview_gain(&self, gain: f32) { let _ = self.send(Cmd::PreviewGain(gain)); }

    pub fn invalidate_plugin_slots(&self, slots: &[(TrackId, usize)]) {
        let mut side = self.side();
        for slot in slots { side.forget(*slot); }
    }

    pub fn take_plugin_load_failures(&self) -> Vec<(String, String)> { std::mem::take(&mut self.side().load_failures) }

    pub fn retry_plugin(&self, id: &str, session: Arc<Session>) {
        {
            let mut side = self.side();
            let keys: Vec<_> = side.shadow.iter().filter(|(_, (plugin, _, created))| plugin == id && !*created).map(|(key, _)| *key).collect();
            for key in keys { side.forget(key); }
            side.load_failures.retain(|(plugin, _)| plugin != id);
        }
        self.update_session(session);
    }

    pub fn plugin_routing_info(&self, track: TrackId, slot: usize) -> (bool, usize) {
        self.side().layouts.get(&(track, slot)).copied().unwrap_or((false, 1))
    }

    /// Opens the plugin's own editor window. Main thread only.
    pub fn open_editor(&self, track: TrackId, slot: usize) -> Result<(), String> {
        self.side().open_editor((track, slot))
    }

    pub fn close_editor(&self, track: TrackId, slot: usize) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.close();
        }
    }

    /// Closes every plugin editor window (main thread). Call before replacing the player: its
    /// instances may be dropped on the audio thread, where editor windows cannot be closed.
    pub fn close_editors(&self) {
        for e in self.side().editors.values_mut() {
            e.close();
        }
    }

    pub fn editor_open(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.get(&(track, slot)).is_some_and(|e| e.is_open())
    }

    pub fn any_editor_open(&self) -> bool {
        self.side().editors.values().any(|e| e.is_open())
    }

    /// Shows a parameter change made elsewhere in the plugin's open editor.
    pub fn editor_set_param(&self, track: TrackId, slot: usize, id: &str, value: f32) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_param(id, value);
        }
    }

    /// Start playback at `from`, stopping at `end`, or looping `looped`.
    pub fn play(&self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.shared.position.store(from, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        let _ = self.send(Cmd::Play { from, end, looped });
    }

    /// Feed live input (from a `record::Recorder`) into the mix for input monitoring.
    pub fn set_input(&self, ring: Arc<record::InputRing>) {
        ring.align_clocks(self.device_buffer.map_or(BLOCK, |frames| frames as usize), self.device_rate);
        let buffer = vec![vec![0.0; BLOCK]; ring.channels.load(Ordering::Relaxed).max(1)];
        let _ = self.send(Cmd::ReadyInput(Box::new(PreparedInput { ring: Some(ring), buffer })));
    }

    pub fn audition(&self, track: TrackId, note: u8) {
        let _ = self.send(Cmd::Audition(track, note));
    }

    pub fn set_recording(&self, on: bool) {
        if on { self.shared.record_clock.nanos.store(0, Ordering::Release); }
        let _ = self.send(Cmd::Recording(on));
    }

    pub fn stop(&self) {
        self.side().collect_garbage();
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.send(Cmd::Stop);
    }

    pub fn locate(&self, at: Samples) {
        self.shared.position.store(at, Ordering::Relaxed);
        let _ = self.send(Cmd::Locate(at));
    }

    pub fn set_speed(&self, speed: f32) {
        let v = if speed.is_finite() { (speed.clamp(0.1, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.speed.store(v, Ordering::Relaxed);
    }

    /// Playback rate last requested, 1.0 at full speed.
    pub fn speed(&self) -> f32 {
        self.shared.speed.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn recording_position_at(&self, nanos: u64) -> Option<Samples> {
        let anchor = self.shared.record_clock.nanos.load(Ordering::Acquire);
        if anchor == 0 { return None; }
        let position = self.shared.record_clock.position.load(Ordering::Relaxed);
        let rate = self.shared.record_clock.rate.load(Ordering::Relaxed);
        let offset = (i128::from(nanos) - i128::from(anchor)) * i128::from(rate) / 1_000_000_000;
        Some(position.saturating_add(offset.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64))
    }

    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn is_auditioning(&self) -> bool {
        self.shared.auditioning.load(Ordering::Relaxed)
    }

    pub fn meters(&self) -> MeterSnapshot {
        self.shared.meters.snapshot()
    }

    pub fn copy_meters(&self, out: &mut MeterSnapshot) {
        if let Some(source) = self.shared.meters.try_snapshot() {
            copy_meter_snapshot(&source, out);
        }
    }
}

fn copy_meter_snapshot(source: &MeterSnapshot, out: &mut MeterSnapshot) {
    out.tracks.retain(|id, _| source.tracks.contains_key(id));
    for (id, meter) in &source.tracks {
        out.tracks.entry(*id).or_default().copy_from(meter);
    }
    out.main.copy_from(&source.main);
}

impl Side {
    fn capture_states(&mut self) -> PluginStates {
        self.states.iter_mut().filter(|(k, _)| !self.refused.contains(*k)).filter_map(|(&(t, slot), r)| Some((t, slot, r.save_state()?))).collect()
    }

    fn open_editor(&mut self, key: (TrackId, usize)) -> Result<(), String> {
        let e = self.editors.get_mut(&key).ok_or_else(|| "this plugin has no editor (or it is not loaded)".to_string())?;
        e.open()?;
        self.refused.remove(&key);
        Ok(())
    }

    /// Creates the third-party instances `s` needs that the audio thread does not have yet.
    fn prepare(&mut self, s: &Session) -> Vec<PreparedInstance> {
        self.collect_garbage();
        let sr = s.sample_rate.as_f64() as f32;
        if (sr - self.rate).abs() > 0.5 {
            // The audio thread rebuilds its engine: everything is created anew.
            let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
            for k in keys {
                self.forget(k);
            }
            self.rate = sr;
        }
        let specs = soundcraft_mix::instance_specs(s);
        let stale: Vec<(TrackId, usize)> = self.shadow.keys().filter(|k| !specs.iter().any(|x| (x.track, x.slot) == **k)).copied().collect();
        for k in stale {
            self.forget(k);
        }
        let mut out = Vec::new();
        for spec in specs {
            let key = (spec.track, spec.slot);
            if self.shadow.get(&key).is_some_and(|(id, ch, _)| *id == spec.id && *ch == spec.ch) {
                continue;
            }
            self.forget(key);
            let made = soundcraft_mix::create_instance(&spec.id, spec.state.as_deref(), sr, BLOCK, spec.ch);
            log::debug!("plugin instance for {:?} slot {}: {} ({} ch, created: {})", spec.track, spec.slot, spec.id, spec.ch, made.is_some());
            self.shadow.insert(key, (spec.id.clone(), spec.ch, made.is_some()));
            if made.is_none() { self.load_failures.push((spec.id.clone(), "The plugin could not be instantiated. Check its authorization, installation and compatibility, then retry.".into())); }
            if let Some(soundcraft_mix::NewInstance { mut plugin, refused_state }) = made {
                self.layouts.insert(key, (plugin.has_sidechain(), plugin.output_bus_count().min(16)));
                if let Some(e) = plugin.editor() {
                    self.editors.insert(key, e);
                }
                if let Some(r) = plugin.state_reader() {
                    self.states.insert(key, r);
                }
                if refused_state {
                    self.refused.insert(key);
                }
                out.push(PreparedInstance { track: spec.track, slot: spec.slot, id: spec.id, ch: spec.ch, plugin });
            }
        }
        out
    }
}

/// ITU stereo fold-down gains for each main channel of a session.
fn fold_gains(s: &Session) -> [(f32, f32); MAX_CHANNELS] {
    let mut g = [(0.0, 0.0); MAX_CHANNELS];
    let f = s.main_format();
    let sp = f.speakers();
    if sp.len() == f.channels() {
        for (o, x) in g.iter_mut().zip(sp.iter()) {
            *o = itu_stereo_fold(*x);
        }
    } else {
        // Ambisonics: W only, at -3 dB per side.
        g[0] = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    }
    g
}

/// The output stream: the mix renders f32 into a scratch buffer allocated here (so the callback
/// never allocates), converted to the device's sample type.
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: AudioState,
    channels: usize,
    fault: &Arc<Fault>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let frame = channels.max(1);
    let mut scratch = vec![0.0f32; 8192 * frame];
    let chunk = scratch.len();
    let (failed, guard, gone) = (Arc::clone(fault), Arc::clone(shared), Arc::clone(fault));
    device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            mark_audio_thread();
            for out in data.chunks_mut(chunk) {
                let Some(s) = scratch.get_mut(..out.len()) else { return };
                guarded_block(&failed, &guard, s, |d| {
                    state.drain();
                    if state.playing && state.recording && state.shared.record_clock.nanos.load(Ordering::Acquire) == 0 {
                        let timestamp = info.timestamp();
                        let delay = timestamp.playback.duration_since(&timestamp.callback).map_or(0, |duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64);
                        let first = state.pos.saturating_sub(state.pending_len as i64).saturating_add(state.read.floor() as i64);
                        state.shared.record_clock.position.store(first, Ordering::Relaxed);
                        state.shared.record_clock.rate.store(state.session.sample_rate.hz(), Ordering::Relaxed);
                        state.shared.record_clock.nanos.store(record::clock_nanos().saturating_add(delay).max(1), Ordering::Release);
                    }
                    state.fill(d, channels);
                    state.fill_preview(d, channels);
                });
                for (o, x) in out.iter_mut().zip(s.iter()) {
                    *o = T::from_sample(*x);
                }
            }
        },
        // cpal may call this on the audio thread (on ALSA in a loop while the device stays
        // broken): marked, so the logger hands the record off instead of writing it.
        move |e: cpal::StreamError| {
            mark_audio_thread();
            if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                gone.gone.store(true, Ordering::Relaxed);
            }
            log::warn!("audio stream error: {e}");
        },
        None,
    )
}

/// Open the output device `cfg` chooses (else the default) at the chosen rate (else the session
/// rate) if possible, in the best sample format it offers (f32, else 32- or 16-bit integers),
/// with as many channels as it offers up to `want_ch` (at least stereo when available), and the
/// chosen buffer size when the device allows it.
fn open_device(want_rate: u32, want_ch: u16, cfg: &AudioConfig) -> Result<(cpal::Device, cpal::StreamConfig, cpal::SampleFormat, String), String> {
    use cpal::traits::DeviceTrait;
    let host = devices::host(cfg);
    let device = devices::device(&host, cfg.output.as_deref(), true).ok_or_else(|| "no output device".to_string())?;
    #[allow(deprecated)]
    let name = device.name().unwrap_or_else(|_| "Audio device".into());
    // Prefer a config at the chosen (else session) rate; else the device default (we resample).
    let want_ch = want_ch.max(2);
    let rates: Vec<u32> = cfg.sample_rate.into_iter().chain(Some(want_rate)).collect();
    let mut chosen: Option<(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)> = None;
    // Better: a preferred format first, then more channels.
    let better = |c: &cpal::SupportedStreamConfigRange, x: &(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)| {
        let (rc, rx) = (devices::format_rank(c.sample_format()).unwrap_or(usize::MAX), devices::format_rank(x.2).unwrap_or(usize::MAX));
        rc < rx || (rc == rx && c.channels() > x.0.channels)
    };
    if let Ok(configs) = device.supported_output_configs() {
        let configs: Vec<cpal::SupportedStreamConfigRange> = configs.collect();
        for rate in rates {
            for c in &configs {
                if devices::format_rank(c.sample_format()).is_some()
                    && c.min_sample_rate().0 <= rate
                    && c.max_sample_rate().0 >= rate
                    && c.channels() >= 2
                    && c.channels() <= want_ch
                    && chosen.as_ref().is_none_or(|x| better(c, x))
                {
                    chosen = Some((c.with_sample_rate(cpal::SampleRate(rate)).config(), *c.buffer_size(), c.sample_format()));
                }
            }
            if chosen.is_some() {
                break;
            }
        }
    }
    let (config, format) = match chosen {
        Some((mut c, buffers, format)) => {
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
        None => {
            let d = device.default_output_config().map_err(|e| e.to_string())?;
            if devices::format_rank(d.sample_format()).is_none() {
                return Err(format!("unsupported device sample format {:?}", d.sample_format()));
            }
            let buffers = *d.buffer_size();
            let format = d.sample_format();
            let mut c = d.config();
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
    };
    Ok((device, config, format, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_meter_snapshot_updates_and_drops_stale_tracks() {
        let keep = TrackId(1);
        let stale = TrackId(2);
        let mut source = MeterSnapshot::default();
        source.tracks.insert(keep, StripMeter { peak: [0.5, 0.25], peaks: vec![0.5, 0.25], gain_reduction_db: 3.0, ..StripMeter::default() });
        source.main.peaks = vec![0.75, 0.5];
        let mut out = MeterSnapshot::default();
        out.tracks.insert(stale, StripMeter::default());
        copy_meter_snapshot(&source, &mut out);
        assert_eq!(out.tracks.len(), 1);
        assert_eq!(out.tracks[&keep], source.tracks[&keep]);
        assert_eq!(out.main, source.main);
    }

    #[test]
    fn a_block_that_panics_goes_silent_and_is_reported_once() {
        let shared = Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(true), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        };
        let fault = Fault::default();
        let mut out = [0.5f32; 8];
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!((out, fault.trouble(false)), ([0.25; 8], None));
        guarded_block(&fault, &shared, &mut out, |_| std::panic::panic_any("a bug in the mix"));
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!(out, [0.0; 8]);
        assert!(!shared.playing.load(Ordering::Relaxed));
        assert_eq!((fault.trouble(false), fault.trouble(false)), (Some(OutputTrouble::Bug), None));
        assert_eq!(fault.beats.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_lost_or_stalled_device_is_reported_once() {
        let ago = |seen: &mut Option<(u64, std::time::Instant)>| {
            if let Some(s) = seen {
                s.1 = std::time::Instant::now().checked_sub(STALL).unwrap();
            }
        };
        let mut seen = None;
        assert!(!stalled(&mut seen, 5, true));
        ago(&mut seen);
        assert!(stalled(&mut seen, 5, true));
        assert!(!stalled(&mut seen, 6, true), "it asked again");
        ago(&mut seen);
        assert!(!stalled(&mut seen, 6, false), "stopped, so nothing to ask for");
        assert_eq!(Fault::default().trouble(true), Some(OutputTrouble::Stalled));
        let fault = Fault::default();
        fault.gone.store(true, Ordering::Relaxed);
        assert_eq!((fault.trouble(false), fault.trouble(true)), (Some(OutputTrouble::Gone), None));
    }

    #[test]
    fn audio_state_plays_and_stops_at_end() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(shared.position.load(Ordering::Relaxed) >= 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn short_device_callbacks_render_short_mix_blocks() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 64];
        st.fill(&mut out, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn surround_main_maps_to_device_channels_or_folds_down() {
        use soundcraft_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
        let mut s = soundcraft_model::Session::default();
        s.outputs[0].format = ChannelFormat::Surround51;
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Surround51, None);
        let chans: Vec<Vec<f32>> = (0..6).map(|c| vec![0.1 * (c + 1) as f32; 48_000]).collect();
        s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 48_000));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&s), 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        // An 8-channel device: channels in order, the rest silent.
        let mut out = vec![0.0f32; 8 * 256];
        st.fill(&mut out, 8);
        let f = &out[8 * 100..8 * 101];
        for c in 0..6 {
            assert!((f[c] - 0.1 * (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
        assert!(f[6] == 0.0 && f[7] == 0.0);
        // A stereo device: ITU fold-down.
        let mut out = vec![0.0f32; 2 * 256];
        st.fill(&mut out, 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let l = (0.1 + h * 0.3 + h * 0.5f32).min(1.0);
        assert!((out[200] - l).abs() < 1e-4, "{} vs {l}", out[200]);
        assert_eq!(shared.meters.snapshot().main.peaks.len(), 6);
    }

    #[test]
    fn instances_arrive_with_the_session_and_leave_through_the_return_channel() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let plugin = soundcraft_dsp::create("gain").unwrap();
        tx.send(Cmd::Session(Arc::clone(&s), vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin }])).unwrap();
        let mut out = vec![0.0f32; 256];
        st.fill(&mut out, 2);
        assert!(grx.try_recv().is_err(), "nothing retired yet");
        // A session without the insert retires the instance to the return channel.
        tx.send(Cmd::Session(Arc::new(soundcraft_model::Session::default()), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
        // A sample-rate change rebuilds the engine without dropping plugins on this thread.
        tx.send(Cmd::Session(
            Arc::clone(&s),
            vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin: soundcraft_dsp::create("gain").unwrap() }],
        ))
        .unwrap();
        st.fill(&mut out, 2);
        let mut s44 = (*s).clone();
        s44.sample_rate = soundcraft_time::SampleRate::new(44_100).unwrap();
        tx.send(Cmd::Session(Arc::new(s44), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
    }

    #[test]
    fn plugin_states_are_read_without_the_audio_thread() {
        let mut side = Side::new(48_000.0, channel().1);
        let (a, b) = (TrackId(1), TrackId(2));
        side.states.insert((b, 0), Box::new(|| Some(b"b0".to_vec())));
        side.states.insert((a, soundcraft_mix::INSTRUMENT_SLOT), Box::new(|| Some(b"synth".to_vec())));
        side.states.insert((a, 3), Box::new(|| Some(b"a3".to_vec())));
        side.states.insert((a, 4), Box::new(|| None));
        let want = vec![(a, 3, b"a3".to_vec()), (a, soundcraft_mix::INSTRUMENT_SLOT, b"synth".to_vec()), (b, 0, b"b0".to_vec())];
        assert_eq!(side.capture_states(), want);
        side.forget((a, 3));
        assert_eq!(side.capture_states(), want[1..]);
    }

    struct Window;

    impl PluginEditor for Window {
        fn open(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {}
        fn is_open(&self) -> bool {
            false
        }
        fn idle(&mut self) -> Vec<(String, f32)> {
            Vec::new()
        }
    }

    #[test]
    fn a_refused_state_is_kept_until_the_editor_opens() {
        let mut side = Side::new(48_000.0, channel().1);
        let k = (TrackId(1), 0);
        side.states.insert(k, Box::new(|| Some(b"defaults".to_vec())));
        side.editors.insert(k, Box::new(Window));
        side.refused.insert(k);
        assert!(side.capture_states().is_empty());
        side.open_editor(k).unwrap();
        assert_eq!(side.capture_states(), vec![(TrackId(1), 0, b"defaults".to_vec())]);
    }

    #[test]
    fn side_creates_each_missing_instance_once() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut side = Side::new(48_000.0, channel().1);
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:does.not.exist"));
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("gain"));
        assert!(side.prepare(&s).is_empty(), "the plugin is not installed; built-ins stay with the engine");
        assert_eq!(side.shadow.get(&(t, 0)), Some(&("clap:does.not.exist".to_string(), 1, false)));
        assert!(side.prepare(&s).is_empty());
        assert_eq!(side.shadow.len(), 1, "failures are remembered, not retried every update");
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        side.prepare(&s);
        assert!(side.shadow.is_empty());
    }

    #[test]
    fn replaced_sessions_are_freed_on_the_ui_thread() {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, shared, Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let mut side = Side::new(48_000.0, grx);
        side.return_old_sessions(&tx);
        let mut out = vec![0.0f32; 256];
        let mut watched = Vec::new();
        for _ in 0..3 * OLD_SESSIONS {
            let s = Arc::new(soundcraft_model::Session::default());
            watched.push(Arc::downgrade(&s));
            side.collect_garbage();
            tx.send(Cmd::Session(s, Vec::new())).unwrap();
            st.fill(&mut out, 2);
            let replaced = watched.len().checked_sub(2).and_then(|i| watched.get(i));
            assert!(replaced.is_none_or(|w| w.strong_count() == 1), "freed on the audio thread");
        }
        side.collect_garbage();
        assert_eq!(watched.iter().filter(|w| w.strong_count() > 0).count(), 1, "only the live snapshot is left");
    }

    #[test]
    fn looping_wraps() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: Some(Range::new(0, 1000)) }).unwrap();
        let mut out = vec![0.0f32; 20_000];
        st.fill(&mut out, 2);
        let p = shared.position.load(Ordering::Relaxed);
        assert!(p <= 1000, "position {p} should stay inside the loop");
        assert!(shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn locate_while_playing_drops_the_old_position_audio() {
        use soundcraft_model::{ChannelFormat, Clip, Insert, SourceAudio, SourceId, TrackKind};
        for (plugin, params) in [("compressor", &[("lookahead", 5.0), ("ratio", 1.0)][..]), ("room_reverb", &[][..])] {
            let mut s = soundcraft_model::Session::default();
            s.edit.click = false;
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            s.pool.insert(
                SourceId(5),
                Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 4800]] })),
            );
            let id = s.new_clip_id();
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 4800));
            let mut ins = Insert::new(plugin);
            for (k, v) in params {
                ins.params.insert((*k).into(), *v);
            }
            tr.mixer.inserts[0] = Some(ins);
            let shared = Arc::new(Shared {
                position: AtomicI64::new(0),
                playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
                speed: AtomicU32::new(1000),
                meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
            });
            let (tx, rx) = channel();
            let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
            tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
            let mut out = vec![0.0f32; 2 * 512];
            let mut before = 0.0f32;
            for _ in 0..9 {
                st.fill(&mut out, 2);
                before = out.iter().fold(before, |m, x| m.max(x.abs()));
            }
            assert!(before > 0.1, "{plugin}: the clip plays before the locate");
            tx.send(Cmd::Locate(1_000_000)).unwrap();
            for _ in 0..8 {
                st.fill(&mut out, 2);
                assert!(out.iter().all(|x| x.abs() < 1e-6), "{plugin}: audio from before the locate");
            }
        }
    }

    #[test]
    fn explicitly_monitored_tracks_are_heard_and_metered_after_playback_stops() {
        use soundcraft_model::{ChannelFormat, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;
        s.track_mut(t).unwrap().mixer.input_monitor = true;
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
        let ring = record::InputRing::new(1, 48_000);
        tx.send(Cmd::Input(Arc::clone(&ring))).unwrap();
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(!shared.playing.load(Ordering::Relaxed));
        ring.push(&[0.5; 4096]);
        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 0.1), "the input is heard");
        assert!(shared.meters.snapshot().tracks.get(&t).is_some_and(|m| m.peak[0] > 0.1), "and metered");
    }
}

#[cfg(test)]
mod audition_transport_tests {
    use super::*;

    #[test]
    fn audition_uses_stopped_monitor_mode_and_transport_commands_clear_its_timer() {
        let mut session = Session::default();
        let track = session.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, None);
        session.track_mut(track).unwrap().instrument = Some(soundcraft_model::Insert::new("drum_rack"));
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0), playing: AtomicBool::new(false), auditioning: AtomicBool::new(false), speed: AtomicU32::new(1000),
            meters: MeterMailbox::default(),
            session_pending: AtomicBool::new(false),
            record_clock: RecordClock::default(),
        });
        let (sender, receiver) = channel();
        let mut state = AudioState::new(receiver, shared, Arc::new(session), 48_000.0, channel().0);
        sender.send(Cmd::Audition(track, 36)).unwrap();
        state.drain();
        assert!(state.mix.monitor_only);
        assert!(state.audition_frames > 0);
        sender.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        state.drain();
        assert_eq!(state.audition_frames, 0);
        assert!(state.mix.audition_track.is_none());
        sender.send(Cmd::Audition(track, 36)).unwrap();
        state.drain();
        assert!(!state.mix.monitor_only);
        sender.send(Cmd::Locate(4800)).unwrap();
        state.drain();
        assert_eq!(state.audition_frames, 0);
        assert!(state.mix.audition_track.is_none());
    }
}

#[cfg(test)]
mod long_preview_lifetime_tests {
    use super::*;

    #[test]
    fn tuned_previews_follow_the_device_rate_and_short_hits_do_not_cut_off_long_hits() {
        let mut session = Session::default();
        let track = session.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, None);
        let long = soundcraft_model::SourceId(800);
        let short = soundcraft_model::SourceId(801);
        session.pool.insert(long, Arc::new(soundcraft_model::SourceAudio::new(soundcraft_audio_io::AudioBuffer {
            sample_rate:24_000, channels:vec![vec![0.25;72_000]],
        })));
        session.pool.insert(short, Arc::new(soundcraft_model::SourceAudio::new(soundcraft_audio_io::AudioBuffer {
            sample_rate:48_000, channels:vec![vec![0.25;4800]],
        })));
        let mut rack = soundcraft_model::Insert::new("drum_rack");
        rack.pad_samples.insert(36,long);
        rack.pad_samples.insert(37,short);
        rack.params.insert("pad36.tune".into(),-12.0);
        session.track_mut(track).unwrap().instrument=Some(rack);
        let shared=Arc::new(Shared {
            position:AtomicI64::new(0), playing:AtomicBool::new(false), auditioning:AtomicBool::new(false),
            speed:AtomicU32::new(1000), meters:MeterMailbox::default(), session_pending:AtomicBool::new(false), record_clock:RecordClock::default(),
        });
        let (sender,receiver)=channel();
        let mut state=AudioState::new(receiver,shared.clone(),Arc::new(session),44_100.0,channel().0);
        sender.send(Cmd::Audition(track,36)).unwrap();
        state.drain();
        assert_eq!(state.audition_frames,264_600);
        assert!(shared.auditioning.load(Ordering::Relaxed));
        state.audition_frames=44_100;
        sender.send(Cmd::Audition(track,37)).unwrap();
        state.drain();
        assert_eq!(state.audition_frames,44_100);
        sender.send(Cmd::Stop).unwrap();
        state.drain();
        assert!(!shared.auditioning.load(Ordering::Relaxed));
    }
}



#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod sample_browser_preview_tests {
    use super::*;

    #[test]
    fn sample_preview_resamples_without_moving_transport_and_returns_retired_audio() {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(1234), playing: AtomicBool::new(false), auditioning: AtomicBool::new(false),
            speed: AtomicU32::new(1000), meters: MeterMailbox::default(), session_pending: AtomicBool::new(false), record_clock: RecordClock::default(),
        });
        let (sender, receiver) = channel();
        let mut state = AudioState::new(receiver, Arc::clone(&shared), Arc::new(Session::default()), 48_000.0, channel().0);
        let (retire, retired) = sync_channel(1);
        state.preview_retirement = Some(retire);
        let audio = Arc::new(soundcraft_model::SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 24_000, channels: vec![vec![0.5; 4800]] }));
        let weak = Arc::downgrade(&audio);
        sender.send(Cmd::Preview(Some(audio))).unwrap();
        state.drain();
        let mut output = vec![0.0; 1920];
        state.fill_preview(&mut output, 2);
        assert_eq!(state.preview_position, 480.0);
        assert!(output.iter().any(|sample| *sample > 0.0));
        assert_eq!(shared.position.load(Ordering::Relaxed), 1234);
        assert!(!shared.playing.load(Ordering::Relaxed));
        sender.send(Cmd::Preview(None)).unwrap();
        state.drain();
        assert!(weak.upgrade().is_some());
        state.drain();
        let returned = retired.try_recv().unwrap();
        assert!(weak.upgrade().is_some());
        drop(returned);
        assert!(weak.upgrade().is_none());
    }
}
