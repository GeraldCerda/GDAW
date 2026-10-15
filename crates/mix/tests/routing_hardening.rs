#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soundcraft_audio_io::AudioBuffer;
use soundcraft_dsp::{Plugin, PluginInfo};
use soundcraft_mix::{MixEngine, PreparedInstance, render_bus, render_range};
use soundcraft_model::{AutoParam, ChannelFormat, Clip, Insert, Route, SendSlot, Session, SourceAudio, SourceId, TrackId, TrackKind};
use soundcraft_time::Range;
use std::sync::Arc;

fn source(samples: Vec<f32>) -> (Session, TrackId) {
    let mut session = Session::default();
    session.edit.delay_compensation = true;
    let length = samples.len() as i64;
    let audio = SourceId(9001);
    session.pool.insert(audio, Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![samples] })));
    let track = session.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Source"));
    let clip = session.new_clip_id();
    let source = session.track_mut(track).unwrap();
    source.mixer.pan = vec![-1.0];
    source.playlist_mut().unwrap().clips.push(Clip::audio(clip, "Source", audio, 0, 0, length));
    (session, track)
}

fn assert_equal_audio(left: &[Vec<f32>], right: &[Vec<f32>]) {
    assert_eq!(left.len(), right.len());
    for (left, right) in left.iter().zip(right) {
        assert_eq!(left.len(), right.len());
        for (index, (left, right)) in left.iter().zip(right).enumerate() {
            assert!((left - right).abs() < 1e-6, "sample {index}: {left} versus {right}");
        }
    }
}

#[test]
fn fader_and_pan_breakpoints_are_independent_of_render_block_size() {
    let (mut session, track) = source(vec![0.25; 512]);
    let track = session.track_mut(track).unwrap();
    for (at, value) in [(0, 0.0), (128, -12.0), (256, 0.0)] {
        track.lane_mut(&AutoParam::Volume).set_point(at, value);
    }
    for (at, value) in [(0, -1.0), (256, 1.0)] {
        track.lane_mut(&AutoParam::Pan(0)).set_point(at, value);
    }
    let small = render_range(&session, Range::new(0, 512), 16);
    let large = render_range(&session, Range::new(0, 512), 512);
    assert_equal_audio(&small, &large);
    let expected = 0.25 * soundcraft_dsp::db_to_gain(-12.0) * std::f32::consts::FRAC_1_SQRT_2;
    assert!((large[0][128] - expected).abs() < 1e-6);
    assert!(large[0][256].abs() < 1e-6);
    assert!((large[1][256] - 0.25).abs() < 1e-6);
}

#[test]
fn mute_changes_at_its_exact_sample_inside_a_block() {
    let (mut session, track) = source(vec![0.25; 512]);
    let lane = session.track_mut(track).unwrap().lane_mut(&AutoParam::Mute);
    lane.set_point(0, 0.0);
    lane.set_point(128, 1.0);
    lane.set_point(129, 0.0);
    let output = render_range(&session, Range::new(0, 512), 512);
    assert_eq!(output[0][127], 0.25);
    assert_eq!(output[0][128], 0.0);
    assert_eq!(output[0][129], 0.25);
}

#[test]
fn send_level_and_mute_follow_internal_breakpoints() {
    let (mut session, track) = source(vec![0.25; 512]);
    let bus = session.add_bus("Return", ChannelFormat::Mono);
    let source = session.track_mut(track).unwrap();
    let mut send = SendSlot::new(Route::Bus(bus));
    send.level_db = 0.0;
    send.pre_fader = true;
    source.mixer.sends[0] = Some(send);
    for (at, value) in [(0, 0.0), (128, -12.0), (256, 0.0)] {
        source.lane_mut(&AutoParam::SendLevel(0)).set_point(at, value);
    }
    for (at, value) in [(0, 0.0), (160, 1.0), (192, 0.0)] {
        source.lane_mut(&AutoParam::SendMute(0)).set_point(at, value);
    }
    let aux = session.add_track(TrackKind::Aux, ChannelFormat::Mono, Some("Return"));
    let aux = session.track_mut(aux).unwrap();
    aux.mixer.input = Route::Bus(bus);
    aux.mixer.pan = vec![1.0];
    let small = render_range(&session, Range::new(0, 512), 16);
    let large = render_range(&session, Range::new(0, 512), 512);
    assert_equal_audio(&small, &large);
    assert!((large[1][128] - 0.25 * soundcraft_dsp::db_to_gain(-12.0)).abs() < 1e-6);
    assert_eq!(large[1][160], 0.0);
    assert!(large[1][192] > 0.0);
}

#[test]
fn serial_aux_paths_and_bus_stems_keep_the_impulse_on_the_timeline() {
    let mut samples = vec![0.0; 4096];
    samples[400] = 0.25;
    let (mut session, track) = source(samples);
    let buses: Vec<_> = (0..12).map(|index| session.add_bus(&format!("Stage {index}"), ChannelFormat::Mono)).collect();
    let mut latency = Insert::new("compressor");
    latency.params.insert("lookahead".into(), 1.0);
    latency.params.insert("ratio".into(), 1.0);
    let source = session.track_mut(track).unwrap();
    source.mixer.inserts[0] = Some(latency.clone());
    let mut send = SendSlot::new(Route::Bus(buses[0]));
    send.level_db = 0.0;
    source.mixer.sends[0] = Some(send);
    for index in (0..12).rev() {
        let aux = session.add_track(TrackKind::Aux, ChannelFormat::Mono, Some(&format!("Aux {index}")));
        let aux = session.track_mut(aux).unwrap();
        aux.mixer.input = Route::Bus(buses[index]);
        aux.mixer.output = if index == 11 { Route::Main } else { Route::Bus(buses[index + 1]) };
        aux.mixer.pan = vec![1.0];
        aux.mixer.inserts[0] = Some(latency.clone());
    }
    for block in [32, 512] {
        let output = render_range(&session, Range::new(0, 4096), block);
        for channel in &output {
            let positions: Vec<_> = channel.iter().enumerate().filter(|(_, value)| value.abs() > 1e-5).map(|(index, _)| index).collect();
            assert_eq!(positions, [400], "block {block}");
        }
        let stem = render_bus(&session, buses[6], Range::new(0, 4096), block);
        let positions: Vec<_> = stem[0].iter().enumerate().filter(|(_, value)| value.abs() > 1e-5).map(|(index, _)| index).collect();
        assert_eq!(positions, [400], "stem block {block}");
    }
}

struct HardClip;

impl Plugin for HardClip {
    fn info(&self) -> &'static PluginInfo { soundcraft_dsp::plugin_info("gain").unwrap() }
    fn prepare(&mut self, _: f32, _: usize, _: usize) {}
    fn reset(&mut self) {}
    fn set_param(&mut self, _: &str, _: f32) -> bool { false }
    fn param(&self, _: &str) -> Option<f32> { None }
    fn process(&mut self, audio: &mut [Vec<f32>], frames: usize) {
        for channel in audio {
            for sample in channel.iter_mut().take(frames) {
                *sample = sample.clamp(-0.1, 0.1);
            }
        }
    }
}

#[test]
fn master_fader_precedes_its_post_fader_inserts() {
    let (mut session, _) = source(vec![0.5; 512]);
    let master = session.add_track(TrackKind::Master, ChannelFormat::Stereo, Some("Master"));
    session.track_mut(master).unwrap().mixer.volume_db = -6.0;
    session.track_mut(master).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.hard-clip"));
    let mut engine = MixEngine::new(48_000.0, 512);
    engine.set_external_instances(true);
    engine.adopt(vec![PreparedInstance { track: master, slot: 0, id: "clap:test.hard-clip".into(), ch: 2, plugin: Box::new(HardClip) }]);
    let mut output = vec![vec![0.0; 512]; 2];
    engine.render(&session, 0, 512, &mut output);
    assert_eq!(output[0][100], 0.1);
}

#[test]
fn floating_point_summing_keeps_headroom_before_hardware_output() {
    let (mut session, first) = source(vec![0.75; 512]);
    let second = session.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Second"));
    let clip = session.new_clip_id();
    let track = session.track_mut(second).unwrap();
    track.mixer.pan = vec![-1.0];
    track.playlist_mut().unwrap().clips.push(Clip::audio(clip, "Second", SourceId(9001), 0, 0, 512));
    session.track_mut(first).unwrap().mixer.pan = vec![-1.0];
    let output = render_range(&session, Range::new(0, 512), 512);
    assert_eq!(output[0][100], 1.5);
}

