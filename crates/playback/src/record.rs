//! Audio input capture for recording.
//!
//! A [`Recorder`] opens the default input device. While armed, the input callback copies each
//! block into a preallocated lock-free ring (it never locks or allocates), and a writer thread
//! streams the ring into one WAV file per take, rewriting the header every second so a crash leaves
//! a playable file.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use soundcraft_model::TrackId;

/// What [`Recorder::finish`] returns: the takes that hold audio and the first problem met.
pub type Takes = (Vec<(TrackId, PathBuf)>, Option<String>);

/// Frames each block of a [`LiveTake`] sums up.
pub const LIVE_BLOCK: usize = 128;
/// Blocks a live take keeps per channel at most (three hours at 48 kHz); its file records on.
const LIVE_BLOCKS: usize = 1 << 22;

/// A take while it records, summed up for drawing: per channel, the (min, max) of each
/// [`LIVE_BLOCK`] frames so far.
pub type LiveTake = Vec<Vec<(f32, f32)>>;

const RING_SECONDS: usize = 4;
/// How long a recording input may go without a callback before it counts as gone.
const STALL: Duration = Duration::from_secs(2);
const GONE: u8 = 1;
const BROKEN: u8 = 2;

/// Why the input can't go on.
#[derive(Debug, Clone, PartialEq)]
pub enum Trouble {
    /// The device went away or stopped sending audio, or its callback hit a bug.
    Device(String),
    /// Writing a take failed (a full disk, say).
    Disk(String),
}

/// Single-producer single-consumer FIFO of samples (input callback → writer thread). The slots
/// are atomics, so it needs no lock and no `unsafe`.
struct CaptureRing {
    slots: Box<[AtomicU32]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl CaptureRing {
    fn new(capacity: usize) -> CaptureRing {
        let n = capacity.max(2).next_power_of_two();
        CaptureRing { slots: (0..n).map(|_| AtomicU32::new(0)).collect(), mask: n - 1, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }

    /// Queue up to `n` samples (`sample(i)` is the i-th); returns how many fit.
    fn push(&self, n: usize, sample: impl Fn(usize) -> f32) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let n = n.min(self.slots.len().saturating_sub(head.wrapping_sub(self.tail.load(Ordering::Acquire))));
        for i in 0..n {
            if let Some(slot) = self.slots.get(head.wrapping_add(i) & self.mask) {
                slot.store(sample(i).to_bits(), Ordering::Relaxed);
            }
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }

    fn pop_all(&self, out: &mut Vec<f32>) {
        let (tail, head) = (self.tail.load(Ordering::Relaxed), self.head.load(Ordering::Acquire));
        out.extend(
            (0..head.wrapping_sub(tail))
                .map(|i| self.slots.get(tail.wrapping_add(i) & self.mask).map_or(0.0, |s| f32::from_bits(s.load(Ordering::Relaxed)))),
        );
        self.tail.store(head, Ordering::Release);
    }
}

/// An input sample, or silence for NaN and infinities, which would poison the take and the mix.
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

struct Shared {
    armed: AtomicBool,
    capturing: AtomicBool,
    epoch: AtomicUsize,
    pending_silence: AtomicUsize,
    ring: CaptureRing,
    /// Samples lost while the ring was full.
    dropped: AtomicUsize,
    /// Input callbacks so far, to tell a device that stopped sending.
    beats: AtomicU64,
    /// Set from the input stream when it can't go on: `GONE` or `BROKEN`.
    fault: AtomicU8,
    /// The first failure writing a take, until it's reported.
    failed: Mutex<Option<String>>,
}

impl Shared {
    fn new(ring: CaptureRing) -> Shared {
        let (beats, fault, failed) = (AtomicU64::new(0), AtomicU8::new(0), Mutex::new(None));
        Shared {
            armed: AtomicBool::new(false), capturing: AtomicBool::new(false), epoch: AtomicUsize::new(0),
            pending_silence: AtomicUsize::new(0), ring, dropped: AtomicUsize::new(0), beats, fault, failed,
        }
    }

    /// Run the input callback's `work`. After a panic in it, the input is broken and does
    /// nothing more.
    fn guarded(&self, work: impl FnOnce()) {
        self.beats.fetch_add(1, Ordering::Relaxed);
        if self.fault.load(Ordering::Relaxed) != BROKEN && std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
            self.fault.store(BROKEN, Ordering::Relaxed);
        }
    }

    /// The input callback's work while armed. Samples that don't fit are owed (`owed`) and queued
    /// as silence before the next block, so the take keeps its timing.
    fn capture(&self, data: &[f32], owed: &mut usize) {
        *owed -= self.ring.push(*owed, |_| 0.0);
        let kept = if *owed == 0 { self.ring.push(data.len(), |i| data.get(i).copied().map_or(0.0, finite)) } else { 0 };
        *owed = owed.saturating_add(data.len() - kept);
        self.pending_silence.store(*owed, Ordering::Release);
        self.dropped.fetch_add(data.len() - kept, Ordering::Relaxed);
    }
}

struct CaptureFlight<'a>(&'a AtomicBool);

impl Drop for CaptureFlight<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub struct Recorder {
    shared: Arc<Shared>,
    /// Live input for monitoring (always fed while the input stream runs).
    pub monitor: Arc<InputRing>,
    input_peaks: Arc<Vec<AtomicU32>>,
    #[cfg(not(target_os = "freebsd"))]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: usize,
    /// The take writer, and what each track's take holds so far.
    writer: Option<(std::thread::JoinHandle<Takes>, Vec<(TrackId, Arc<Mutex<LiveTake>>)>)>,
    /// The callback count last seen, and since when.
    seen: (u64, Instant),
}

/// The input stream: device samples are converted to f32 in a scratch buffer allocated here (so
/// the callback never allocates), then feed the monitor ring and, while armed, the capture.
#[cfg(not(target_os = "freebsd"))]
fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: &Arc<Shared>,
    monitor: &Arc<InputRing>,
    peaks: &Arc<Vec<AtomicU32>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;
    use cpal::traits::DeviceTrait;
    let (sh, errors) = (Arc::clone(shared), Arc::clone(shared));
    let mon = Arc::clone(monitor);
    let levels = Arc::clone(peaks);
    let channels = usize::from(config.channels).max(1);
    let mut block_peaks = vec![0.0f32; channels];
    let mut scratch = vec![0.0f32; 8192 * usize::from(config.channels).max(1)];
    let chunk = scratch.len();
    let mut owed = 0;
    let mut epoch = 0;
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            crate::mark_audio_thread();
            sh.guarded(|| {
                for part in data.chunks(chunk) {
                    let Some(s) = scratch.get_mut(..part.len()) else { return };
                    block_peaks.fill(0.0);
                    for (index, (o, x)) in s.iter_mut().zip(part).enumerate() {
                        let value = finite(f32::from_sample(*x));
                        *o = value;
                        let channel = index % channels;
                        block_peaks[channel] = block_peaks[channel].max(value.abs());
                    }
                    for (level, peak) in levels.iter().zip(&block_peaks) {
                        level.fetch_max(peak.min(1.0).to_bits(), Ordering::Relaxed);
                    }
                    mon.push(s);
                    sh.capturing.store(true, Ordering::SeqCst);
                    let _flight = CaptureFlight(&sh.capturing);
                    if sh.armed.load(Ordering::SeqCst) {
                        let current = sh.epoch.load(Ordering::Acquire);
                        if epoch != current {
                            owed = 0;
                            epoch = current;
                        }
                        sh.capture(s, &mut owed);
                    } else {
                        owed = 0;
                    }
                }
            });
        },
        move |e| {
            // Possibly the audio thread (see `crate::mark_audio_thread`).
            crate::mark_audio_thread();
            if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                let _ = errors.fault.compare_exchange(0, GONE, Ordering::Relaxed, Ordering::Relaxed);
            }
            log::warn!("input stream error: {e}");
        },
        None,
    )
}

fn input_config_rank(channels: u16, format: cpal::SampleFormat) -> (std::cmp::Reverse<u16>, Option<usize>) {
    (std::cmp::Reverse(channels), crate::devices::format_rank(format))
}

impl Recorder {
    /// Open the default input device. Errors when there is none (the caller shows a message).
    pub fn open() -> Result<Recorder, String> {
        Recorder::open_with(&crate::AudioConfig::default())
    }

    /// Open the input device `audio` chooses (else the default) at its chosen rate and buffer
    /// size when the device offers them, else at the device's defaults. Integer devices (ASIO
    /// drivers usually are) are converted to f32.
    #[cfg(not(target_os = "freebsd"))]
    pub fn open_with(audio: &crate::AudioConfig) -> Result<Recorder, String> {
        use cpal::traits::{DeviceTrait, StreamTrait};
        let host = crate::devices::host(audio);
        let device = crate::devices::device(&host, audio.input.as_deref(), false).ok_or("no audio input device")?;
        #[allow(deprecated)]
        let name = device.name().unwrap_or_else(|_| "Input".into());
        let default = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = audio.sample_rate.unwrap_or(default.sample_rate().0);
        let at_rate = device.supported_input_configs().ok().and_then(|configs| {
            configs
                .filter(|config| crate::devices::format_rank(config.sample_format()).is_some() && config.min_sample_rate().0 <= rate && config.max_sample_rate().0 >= rate)
                .min_by_key(|config| input_config_rank(config.channels(), config.sample_format()))
                .map(|config| config.with_sample_rate(cpal::SampleRate(rate)))
        });
        let cfg = match at_rate {
            Some(c) => c,
            None => default,
        };
        let format = cfg.sample_format();
        if crate::devices::format_rank(format).is_none() {
            return Err(format!("unsupported input sample format {format:?}"));
        }
        let mut config = cfg.config();
        config.buffer_size = crate::devices::buffer_size(audio.buffer_size, cfg.buffer_size());
        let channels = usize::from(config.channels).max(1);
        let ring = CaptureRing::new((config.sample_rate.0 as usize).saturating_mul(channels * RING_SECONDS).min(1 << 25));
        let shared = Arc::new(Shared::new(ring));
        let monitor = InputRing::new(usize::from(config.channels), config.sample_rate.0);
        let peaks = Arc::new((0..channels).map(|_| AtomicU32::new(0)).collect::<Vec<_>>());
        let build = |config: &cpal::StreamConfig| match format {
            cpal::SampleFormat::I32 => build_input::<i32>(&device, config, &shared, &monitor, &peaks),
            cpal::SampleFormat::I16 => build_input::<i16>(&device, config, &shared, &monitor, &peaks),
            _ => build_input::<f32>(&device, config, &shared, &monitor, &peaks),
        };
        // The chosen buffer size first; if the device refuses it, its default.
        let stream = match build(&config) {
            Ok(s) => s,
            Err(e) if config.buffer_size != cpal::BufferSize::Default => {
                log::warn!("cannot open the input with buffer {:?}: {e}; using the device default", config.buffer_size);
                config.buffer_size = cpal::BufferSize::Default;
                build(&config).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
        stream.play().map_err(|e| e.to_string())?;
        let seen = (0, Instant::now());
        Ok(Recorder { monitor, input_peaks: peaks, shared, _stream: Some(stream), device_name: name, sample_rate: config.sample_rate.0, channels, writer: None, seen })
    }

    #[cfg(target_os = "freebsd")]
    pub fn open_with(_audio: &crate::AudioConfig) -> Result<Recorder, String> {
        Err("recording is not supported on this platform yet".into())
    }

    pub fn take_input_peaks(&self) -> Vec<f32> {
        self.input_peaks.iter().map(|peak| f32::from_bits(peak.swap(0, Ordering::Relaxed))).collect()
    }

    /// Create each take's file and start recording into them: a track's take records the given
    /// input channels into a new file.
    pub fn arm(&mut self, takes: Vec<(TrackId, Vec<usize>, PathBuf)>) -> Result<(), String> {
        if self.writer.is_some() {
            return Err("already recording".into());
        }
        if takes.is_empty() || takes.iter().any(|(_, inputs, _)| inputs.is_empty() || inputs.iter().any(|&channel| channel >= self.channels)) {
            return Err(format!("recording inputs must select available channels on the {}-channel input device", self.channels));
        }
        let takes =
            takes.into_iter().map(|(id, inputs, path)| TakeWriter::create(id, inputs, path, self.sample_rate)).collect::<Result<Vec<_>, _>>()?;
        let live = takes.iter().map(|t| (t.id, Arc::clone(&t.live))).collect();
        let ring = &self.shared.ring;
        ring.tail.store(ring.head.load(Ordering::Acquire), Ordering::Release);
        *self.shared.failed.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.shared.pending_silence.store(0, Ordering::Release);
        self.shared.epoch.fetch_add(1, Ordering::Release);
        self.shared.armed.store(true, Ordering::SeqCst);
        let (shared, channels) = (Arc::clone(&self.shared), self.channels);
        match std::thread::Builder::new().name("take writer".into()).spawn(move || write_takes(&shared, takes, channels)) {
            Ok(w) => self.writer = Some((w, live)),
            Err(e) => {
                self.shared.armed.store(false, Ordering::SeqCst);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    /// What the take recording onto `track` holds so far.
    pub fn live_take(&self, track: TrackId) -> Option<MutexGuard<'_, LiveTake>> {
        let (_, take) = self.writer.as_ref()?.1.iter().find(|(id, _)| *id == track)?;
        Some(take.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Why the input can't go on, if it can't: the device went away or, while recording, stopped
    /// sending audio; its callback hit a bug; or writing a take failed (reported once).
    pub fn trouble(&mut self) -> Option<Trouble> {
        if let Some(e) = self.shared.failed.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return Some(Trouble::Disk(e));
        }
        let name = &self.device_name;
        match self.shared.fault.load(Ordering::Relaxed) {
            GONE => return Some(Trouble::Device(format!("“{name}” was disconnected"))),
            BROKEN => return Some(Trouble::Device("the audio input hit an internal error (see the log)".into())),
            _ => {}
        }
        let beats = self.shared.beats.load(Ordering::Relaxed);
        if beats != self.seen.0 {
            self.seen = (beats, Instant::now());
        }
        (self.seen.1.elapsed() >= STALL).then(|| Trouble::Device(format!("“{name}” stopped sending audio")))
    }

    /// Input the current take has lost to a slow disk so far, in milliseconds.
    pub fn lost_ms(&self) -> u64 {
        let frames = self.shared.dropped.load(Ordering::Relaxed) / self.channels.max(1);
        (frames as u64).saturating_mul(1000) / u64::from(self.sample_rate.max(1))
    }

    pub fn finish(&mut self) -> Takes {
        self.shared.armed.store(false, Ordering::SeqCst);
        let (takes, problem) = self.writer.take().and_then(|(w, _)| w.join().ok()).unwrap_or_default();
        let lost = self.shared.dropped.swap(0, Ordering::Relaxed) / self.channels.max(1);
        let slow =
            (lost > 0).then(|| format!("{} ms of input lost: the disk was too slow", lost.saturating_mul(1000) / self.sample_rate.max(1) as usize));
        (takes, problem.or(slow))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}

fn write_takes(shared: &Shared, mut takes: Vec<TakeWriter>, channels: usize) -> Takes {
    let (mut block, mut header_written, mut reported) = (Vec::new(), Instant::now(), false);
    loop {
        let stopping = !shared.armed.load(Ordering::SeqCst);
        if stopping && shared.capturing.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        // The ring holds interleaved samples and a pop can end mid-frame: write whole frames and
        // keep the remainder for the next pop, so channels never shift.
        shared.ring.pop_all(&mut block);
        let whole = block.len() - block.len() % channels.max(1);
        takes.iter_mut().for_each(|t| t.write(&block[..whole], channels));
        block.drain(..whole);
        if !reported && let Some(e) = takes.iter().find_map(|t| t.error.clone()) {
            *shared.failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
            reported = true;
        }
        if stopping {
            let mut silence = shared.pending_silence.swap(0, Ordering::Acquire);
            while silence > 0 {
                let count = silence.min(65_536);
                block.resize(block.len().saturating_add(count), 0.0);
                silence -= count;
                let whole = block.len() - block.len() % channels.max(1);
                takes.iter_mut().for_each(|take| take.write(&block[..whole], channels));
                block.drain(..whole);
            }
            break;
        }
        if header_written.elapsed() >= Duration::from_secs(1) {
            header_written = Instant::now();
            takes.iter_mut().for_each(TakeWriter::write_header);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (mut done, mut problem) = (Vec::new(), None);
    for mut t in takes {
        t.write_header();
        if t.frames == 0 {
            let _ = std::fs::remove_file(&t.path);
        } else {
            done.push((t.id, t.path));
        }
        problem = problem.or(t.error);
    }
    (done, problem)
}

/// One take's file. After an error it stops writing; its header keeps covering what was written.
struct TakeWriter {
    id: TrackId,
    inputs: Vec<usize>,
    path: PathBuf,
    file: File,
    rate: u32,
    frames: u64,
    error: Option<String>,
    live: Arc<Mutex<LiveTake>>,
}

impl TakeWriter {
    fn create(id: TrackId, inputs: Vec<usize>, path: PathBuf, rate: u32) -> Result<TakeWriter, String> {
        let file = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| File::create_new(&path));
        let file = file.map_err(|e| format!("{}: {e}", path.display()))?;
        let live = Arc::new(Mutex::new(vec![Vec::new(); inputs.len()]));
        let mut take = TakeWriter { id, inputs, path, file, rate, frames: 0, error: None, live };
        take.write_header();
        take.error.take().map_or(Ok(take), Err)
    }

    /// Append this take's channels of `block` (interleaved, `channels` wide).
    fn write(&mut self, block: &[f32], channels: usize) {
        let channels = channels.max(1);
        if self.error.is_some() {
            return;
        }
        let bytes: Vec<u8> = block
            .chunks_exact(channels)
            .flat_map(|f| self.inputs.iter().flat_map(move |&i| f.get(i).copied().unwrap_or(0.0).to_le_bytes()))
            .collect();
        match self.file.write_all(&bytes) {
            Ok(()) => {
                self.summarize(block, channels);
                self.frames += (block.len() / channels) as u64;
            }
            Err(e) => self.fail(e),
        }
    }

    /// Add this take's channels of `block` to its live take.
    fn summarize(&self, block: &[f32], channels: usize) {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        for (f, n) in block.chunks_exact(channels).zip(self.frames..) {
            let at = (n / LIVE_BLOCK as u64) as usize;
            for (blocks, &i) in live.iter_mut().zip(&self.inputs) {
                let v = f.get(i).copied().unwrap_or(0.0);
                match blocks.get_mut(at) {
                    Some(b) => *b = (b.0.min(v), b.1.max(v)),
                    None if at < LIVE_BLOCKS => blocks.push((v, v)),
                    None => {}
                }
            }
        }
    }

    fn write_header(&mut self) {
        let header = match soundcraft_audio_io::wav_float_header(self.inputs.len(), self.rate, self.frames) {
            Ok(h) => h,
            Err(e) => return self.fail(e),
        };
        let f = &mut self.file;
        if let Err(e) = f.seek(SeekFrom::Start(0)).and_then(|_| f.write_all(&header)).and_then(|()| f.seek(SeekFrom::End(0))) {
            self.fail(e);
        }
    }

    fn fail(&mut self, e: impl std::fmt::Display) {
        self.error.get_or_insert_with(|| format!("{}: {e}", self.path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(path: &PathBuf) -> Vec<Vec<f32>> {
        soundcraft_audio_io::decode(&std::fs::read(path).unwrap(), None).unwrap().1.channels
    }

    #[test]
    fn stopping_after_an_overrun_keeps_the_final_gap_and_channel_alignment() {
        let directory = std::env::temp_dir().join(format!("gdaw-record-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("take.wav");
        let take = TakeWriter::create(TrackId(1), vec![0, 1, 2], path.clone(), 48_000).unwrap();
        let shared = Shared::new(CaptureRing::new(8));
        let mut owed = 0;
        shared.capture(&[0.25; 12], &mut owed);
        assert_eq!(owed, 4);
        let (done, problem) = write_takes(&shared, vec![take], 3);
        assert_eq!((done.len(), problem), (1, None));
        assert_eq!(decode(&path), [vec![0.25, 0.25, 0.25, 0.0], vec![0.25, 0.25, 0.25, 0.0], vec![0.25, 0.25, 0.0, 0.0]]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_full_ring_owes_the_lost_samples_as_silence() {
        let shared = Shared::new(CaptureRing::new(8));
        let (mut owed, mut out) = (0, Vec::new());
        shared.capture(&[1.0; 6], &mut owed);
        shared.capture(&[2.0; 4], &mut owed);
        shared.ring.pop_all(&mut out);
        shared.capture(&[3.0; 2], &mut owed);
        shared.ring.pop_all(&mut out);
        assert_eq!(out, [vec![1.0; 6], vec![2.0; 2], vec![0.0; 2], vec![3.0; 2]].concat());
        assert_eq!((owed, shared.dropped.load(Ordering::Relaxed)), (0, 2));
    }

    #[test]
    fn nan_and_infinite_input_records_as_silence() {
        let shared = Shared::new(CaptureRing::new(8));
        let (mut owed, mut out) = (0, Vec::new());
        shared.capture(&[0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5], &mut owed);
        shared.ring.pop_all(&mut out);
        assert_eq!(out, [0.5, 0.0, 0.0, 0.0, -0.5]);
    }

    fn recorder(shared: &Arc<Shared>) -> Recorder {
        Recorder {
            shared: Arc::clone(shared),
            monitor: InputRing::new(1, 48_000),
            input_peaks: Arc::new(vec![AtomicU32::new(0)]),
            #[cfg(not(target_os = "freebsd"))]
            _stream: None,
            device_name: "Mic In".into(),
            sample_rate: 48_000,
            channels: 1,
            writer: None,
            seen: (0, Instant::now()),
        }
    }

    #[test]
    fn input_peaks_are_independent_and_reset_after_reading() {
        let shared = Arc::new(Shared::new(CaptureRing::new(64)));
        let mut r = recorder(&shared);
        r.input_peaks = Arc::new(vec![AtomicU32::new(0.25f32.to_bits()), AtomicU32::new(0.75f32.to_bits())]);
        assert_eq!(r.take_input_peaks(), [0.25, 0.75]);
        assert_eq!(r.take_input_peaks(), [0.0, 0.0]);
    }

    #[test]
    fn invalid_recording_channels_are_rejected_before_creating_files() {
        let shared = Arc::new(Shared::new(CaptureRing::new(64)));
        let mut r = recorder(&shared);
        let root = std::env::temp_dir().join(format!("soundcraft-invalid-input-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for inputs in [Vec::new(), vec![1], vec![usize::MAX]] {
            assert!(r.arm(vec![(TrackId(1), inputs, root.join("take.wav"))]).is_err());
            assert!(!root.exists());
            assert!(r.writer.is_none());
            assert!(!shared.armed.load(Ordering::Acquire));
        }
    }

    #[test]
    fn a_bug_a_stall_or_a_lost_device_is_trouble() {
        let shared = Arc::new(Shared::new(CaptureRing::new(64)));
        let mut r = recorder(&shared);
        shared.armed.store(true, Ordering::Release);
        assert_eq!(r.trouble(), None);
        r.seen.1 = Instant::now().checked_sub(STALL).unwrap();
        assert_eq!(r.trouble(), Some(Trouble::Device("“Mic In” stopped sending audio".into())));
        shared.guarded(|| {});
        assert_eq!(r.trouble(), None, "it sent again");
        shared.armed.store(false, Ordering::Release);
        r.seen.1 = Instant::now().checked_sub(STALL).unwrap();
        assert_eq!(r.trouble(), Some(Trouble::Device("“Mic In” stopped sending audio".into())));
        shared.guarded(|| {});
        assert_eq!(r.trouble(), None, "silent callbacks still keep an unarmed input healthy");
        shared.guarded(|| std::panic::panic_any("a bug in the input"));
        let mut ran = false;
        shared.guarded(|| ran = true);
        assert!(!ran);
        assert_eq!(r.trouble(), Some(Trouble::Device("the audio input hit an internal error (see the log)".into())));
        let gone = Arc::new(Shared::new(CaptureRing::new(64)));
        gone.fault.store(GONE, Ordering::Relaxed);
        assert_eq!(recorder(&gone).trouble(), Some(Trouble::Device("“Mic In” was disconnected".into())));
    }

    #[test]
    fn a_failed_write_is_trouble_while_still_recording() {
        let dir = std::env::temp_dir().join(format!("soundcraft-failed-take-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut take = TakeWriter::create(TrackId(1), vec![0], dir.join("Vox_01.wav"), 48_000).unwrap();
        take.file = File::open(&take.path).unwrap();
        let shared = Arc::new(Shared::new(CaptureRing::new(1 << 12)));
        shared.armed.store(true, Ordering::Release);
        let mut r = recorder(&shared);
        let sh = Arc::clone(&shared);
        let writer = std::thread::spawn(move || write_takes(&sh, vec![take], 1));
        shared.capture(&[0.5; 480], &mut 0);
        let trouble = (0..300).find_map(|_| {
            std::thread::sleep(Duration::from_millis(10));
            r.trouble()
        });
        assert!(matches!(&trouble, Some(Trouble::Disk(e)) if e.contains("Vox_01.wav")), "{trouble:?}");
        assert_eq!(r.trouble(), None, "reported once");
        shared.armed.store(false, Ordering::Release);
        assert!(writer.join().unwrap().1.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn takes_stream_into_their_files() {
        let dir = std::env::temp_dir().join(format!("soundcraft-takes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gtr = TakeWriter::create(TrackId(1), vec![1, 2], dir.join("Gtr_01.wav"), 48_000).unwrap();
        let mut vox = TakeWriter::create(TrackId(2), vec![0], dir.join("Vox_01.wav"), 48_000).unwrap();
        assert!(TakeWriter::create(TrackId(3), vec![0], dir.join("Vox_01.wav"), 48_000).is_err(), "never overwrites a file");
        vox.write(&[0.5; 300], 3);
        vox.write_header();
        vox.write(&[0.25; 150], 3);
        assert_eq!(decode(&dir.join("Vox_01.wav"))[0].len(), 100, "the header covers what it has seen");

        let shared = Arc::new(Shared::new(CaptureRing::new(1 << 16)));
        shared.armed.store(true, Ordering::Release);
        let sh = Arc::clone(&shared);
        let writer = std::thread::spawn(move || write_takes(&sh, vec![gtr, vox], 3));
        let input: Vec<f32> = (0..3 * 4_800).map(|i| i as f32 / 20_000.0).collect();
        let mut owed = 0;
        input.chunks(3 * 480).for_each(|block| shared.capture(block, &mut owed));
        shared.armed.store(false, Ordering::Release);
        let (done, problem) = writer.join().unwrap();
        let channel = |c: usize| input.iter().skip(c).step_by(3).copied().collect::<Vec<f32>>();
        assert_eq!((done.len(), problem), (2, None));
        assert_eq!(decode(&done[0].1), vec![channel(1), channel(2)]);
        assert_eq!(decode(&done[1].1), vec![[vec![0.5; 100], vec![0.25; 50], channel(0)].concat()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_take_sums_itself_up_for_drawing_while_it_records() {
        let dir = std::env::temp_dir().join(format!("soundcraft-live-take-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut take = TakeWriter::create(TrackId(4), vec![2, 0], dir.join("Gtr_01.wav"), 48_000).unwrap();
        let input: Vec<f32> = (0..300).flat_map(|f| [f as f32, -1.0, -(f as f32)]).collect();
        take.write(&input[..3 * 100], 3);
        take.write(&input[3 * 100..], 3);
        let live = take.live.lock().unwrap().clone();
        assert_eq!(live, [[(-127.0, 0.0), (-255.0, -128.0), (-299.0, -256.0)], [(0.0, 127.0), (128.0, 255.0), (256.0, 299.0)]]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub struct InputRing {
    slots: Box<[AtomicU32]>,
    width: usize,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    phase: AtomicU64,
    dropped: AtomicU64,
    underflows: AtomicU64,
    target: AtomicUsize,
    correction: AtomicU64,
    pub channels: std::sync::atomic::AtomicUsize,
    /// Input device sample rate in Hz (0 when unknown, which reads as "same as the session").
    rate: u32,
}

impl InputRing {
    pub fn new(channels: usize, rate: u32) -> Arc<InputRing> {
        let ch = channels.clamp(1, 256);
        let frames = if rate == 0 { 24_000 } else { rate as usize / 2 };
        let frames = frames.clamp(2, (1 << 22) / ch).next_power_of_two();
        Arc::new(InputRing {
            slots: (0..frames * ch).map(|_| AtomicU32::new(0)).collect(),
            width: ch,
            mask: frames - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            phase: AtomicU64::new(0.0f64.to_bits()),
            dropped: AtomicU64::new(0),
            underflows: AtomicU64::new(0),
            target: AtomicUsize::new(0),
            correction: AtomicU64::new(1.0f64.to_bits()),
            channels: std::sync::atomic::AtomicUsize::new(ch),
            rate,
        })
    }

    pub fn push(&self, samples: &[f32]) {
        let head = self.head.load(Ordering::Relaxed);
        let used = head.wrapping_sub(self.tail.load(Ordering::Acquire));
        let offered = samples.len() / self.width;
        let kept = offered.min((self.mask + 1).saturating_sub(used));
        for (frame, values) in samples.chunks_exact(self.width).take(kept).enumerate() {
            let base = (head.wrapping_add(frame) & self.mask) * self.width;
            for (channel, value) in values.iter().enumerate() {
                if let Some(slot) = self.slots.get(base + channel) {
                    slot.store(finite(*value).to_bits(), Ordering::Relaxed);
                }
            }
        }
        self.head.store(head.wrapping_add(kept), Ordering::Release);
        self.dropped.fetch_add((offered - kept) as u64, Ordering::Relaxed);
    }

    pub fn discard(&self) {
        self.tail.store(self.head.load(Ordering::Acquire), Ordering::Release);
        self.phase.store(0.0f64.to_bits(), Ordering::Relaxed);
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn align_clocks(&self, output_frames: usize, output_rate: u32) {
        let rate = if self.rate == 0 { output_rate } else { self.rate };
        let target = ((output_frames.max(1) as f64 * f64::from(rate) / f64::from(output_rate.max(1))) * 2.0).ceil() as usize;
        self.target.store(target.clamp(2, ((self.mask + 1) / 4).max(2)), Ordering::Relaxed);
        self.correction.store(1.0f64.to_bits(), Ordering::Relaxed);
    }

    pub fn underflow_frames(&self) -> u64 { self.underflows.load(Ordering::Relaxed) }

    /// Fill `out` (planar, at `session_rate`) with up to `frames` frames, converting from the
    /// input rate. Missing frames are silence.
    pub fn pop_into(&self, out: &mut [Vec<f32>], frames: usize, session_rate: u32) {
        let ch = self.width;
        for c in out.iter_mut() {
            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
        }
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let mut available = head.wrapping_sub(tail).min(self.mask + 1);
        let mut phase = f64::from_bits(self.phase.load(Ordering::Relaxed));
        let nominal = if self.rate == 0 || session_rate == 0 { 1.0 } else { f64::from(self.rate) / f64::from(session_rate) };
        let target = self.target.load(Ordering::Relaxed);
        let mut correction = f64::from_bits(self.correction.load(Ordering::Relaxed));
        if target > 0 {
            let error = (available as f64 - target as f64) / target as f64;
            let desired = 1.0 + (error * 0.0005).clamp(-0.002, 0.002);
            let smoothing = (frames as f64 / f64::from(session_rate.max(1))).min(1.0);
            correction += (desired - correction) * smoothing;
        }
        let step = nominal * correction;
        for f in 0..frames {
            let consumed = (phase.floor() as usize).min(available);
            tail = tail.wrapping_add(consumed);
            available -= consumed;
            phase -= consumed as f64;
            if phase >= 1.0 || available == 0 || (phase > 0.0 && available < 2) {
                self.underflows.fetch_add(frames.saturating_sub(f) as u64, Ordering::Relaxed);
                if available == 0 { phase = 0.0; }
                break;
            }
            let fr = phase as f32;
            for c in 0..ch {
                let sample = |at: usize| {
                    self.slots.get((at & self.mask) * ch + c).map_or(0.0, |slot| f32::from_bits(slot.load(Ordering::Relaxed)))
                };
                let a = sample(tail);
                let b = if available > 1 { sample(tail.wrapping_add(1)) } else { a };
                if let Some(x) = out.get_mut(c).and_then(|o| o.get_mut(f)) {
                    *x = a + (b - a) * fr;
                }
            }
            phase += step;
        }
        let consumed = (phase.floor() as usize).min(available);
        tail = tail.wrapping_add(consumed);
        phase -= consumed as f64;
        self.phase.store(phase.to_bits(), Ordering::Relaxed);
        self.correction.store(correction.to_bits(), Ordering::Relaxed);
        self.tail.store(tail, Ordering::Release);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::InputRing;

    #[test]
    fn monitoring_reads_a_final_single_frame_without_lookahead() {
        let ring = InputRing::new(2, 48_000);
        ring.push(&[0.25, -0.5]);
        let mut output = vec![vec![1.0; 2]; 2];
        ring.pop_into(&mut output, 2, 48_000);
        assert_eq!(output, [vec![0.25, 0.0], vec![-0.5, 0.0]]);
    }

    #[test]
    fn monitoring_overflow_keeps_whole_frames_and_recovers_after_discard() {
        let ring = InputRing::new(3, 8);
        let input: Vec<f32> = (0..18).map(|value| value as f32).collect();
        ring.push(&input);
        assert_eq!(ring.dropped_frames(), 2);
        let mut output = vec![vec![0.0; 4]; 3];
        ring.pop_into(&mut output, 4, 8);
        assert_eq!(output, [vec![0.0, 3.0, 6.0, 9.0], vec![1.0, 4.0, 7.0, 10.0], vec![2.0, 5.0, 8.0, 11.0]]);
        ring.push(&[20.0, 21.0, 22.0]);
        ring.discard();
        ring.push(&[30.0, 31.0, 32.0]);
        ring.pop_into(&mut output, 1, 8);
        assert_eq!([output[0][0], output[1][0], output[2][0]], [30.0, 31.0, 32.0]);
    }

    #[test]
    fn monitoring_counters_wrap_without_shifting_channels() {
        use std::sync::atomic::Ordering;
        let ring = InputRing::new(2, 8);
        ring.head.store(usize::MAX - 1, Ordering::Relaxed);
        ring.tail.store(usize::MAX - 1, Ordering::Relaxed);
        ring.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mut output = vec![vec![0.0; 3]; 2];
        ring.pop_into(&mut output, 3, 8);
        assert_eq!(output, [vec![1.0, 3.0, 5.0], vec![2.0, 4.0, 6.0]]);
    }

    /// A 1 kHz sine recorded at 16 kHz must come out as a 1 kHz sine at 48 kHz, across calls.
    #[test]
    fn monitor_converts_input_rate_to_session_rate() {
        let ring = InputRing::new(1, 16_000);
        let input: Vec<f32> = (0..4_000).map(|i| (2.0 * std::f64::consts::PI * 1_000.0 * f64::from(i) / 16_000.0).sin() as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 48]];
        let mut k = 0usize;
        for _ in 0..10 {
            ring.pop_into(&mut out, 48, 48_000);
            for v in &out[0] {
                let want = (2.0 * std::f64::consts::PI * 1_000.0 * k as f64 / 48_000.0).sin() as f32;
                assert!((v - want).abs() < 0.05, "sample {k}: got {v}, want {want}");
                k += 1;
            }
        }
    }

    /// Input faster than the session (96 kHz into 48 kHz): every other frame is read, in order.
    #[test]
    fn monitor_decimates_when_input_is_faster() {
        let ring = InputRing::new(1, 96_000);
        let input: Vec<f32> = (0..1_000).map(|i| i as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 100]];
        ring.pop_into(&mut out, 100, 48_000);
        for (k, v) in out[0].iter().enumerate() {
            assert_eq!(*v, (2 * k) as f32, "output {k}");
        }
    }

    /// Same rate: samples pass through unchanged.
    #[test]
    fn monitor_passes_through_at_equal_rates() {
        let ring = InputRing::new(2, 48_000);
        let input: Vec<f32> = (0..200).map(|i| i as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 50], vec![0.0f32; 50]];
        ring.pop_into(&mut out, 50, 48_000);
        assert_eq!(out[0][0], 0.0);
        assert_eq!(out[1][0], 1.0);
        assert_eq!(out[0][49], 98.0);
    }

    /// Nothing buffered: silence, not stale data.
    #[test]
    fn monitor_underrun_is_silence() {
        let ring = InputRing::new(1, 16_000);
        let mut out = vec![vec![1.0f32; 16]];
        ring.pop_into(&mut out, 16, 48_000);
        assert!(out[0].iter().all(|v| *v == 0.0));
    }

    /// NaN and infinities from the device monitor as silence.
    #[test]
    fn monitor_silences_nan_and_infinite_input() {
        let ring = InputRing::new(1, 48_000);
        ring.push(&[0.5, f32::NAN, f32::INFINITY, -0.5, 0.0, 0.0, 0.0, 0.0]);
        let mut out = vec![vec![1.0f32; 4]];
        ring.pop_into(&mut out, 4, 48_000);
        assert_eq!(out[0], [0.5, 0.0, 0.0, -0.5]);
    }
}

#[cfg(test)]
mod input_config_tests {
    use super::input_config_rank;

    #[test]
    fn physical_channel_coverage_precedes_sample_format_preference() {
        assert!(input_config_rank(12, cpal::SampleFormat::I32) < input_config_rank(2, cpal::SampleFormat::F32));
        assert!(input_config_rank(12, cpal::SampleFormat::F32) < input_config_rank(12, cpal::SampleFormat::I32));
    }
}


