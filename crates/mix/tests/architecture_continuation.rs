#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use soundcraft_audio_io::AudioBuffer;
use soundcraft_mix::{render_range, routing_diagnostics};
use soundcraft_model::{AutoParam, ChannelFormat, Clip, Insert, Route, Session, SourceAudio, SourceId, TrackKind};
use soundcraft_time::Range;
use std::sync::Arc;

fn source() -> (Session, soundcraft_model::TrackId) {
    let mut session = Session::default();
    let track = session.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    let source = SourceId(9001);
    session.pool.insert(source, Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.25; 512]] })));
    let clip = session.new_clip_id();
    let channel = session.track_mut(track).unwrap();
    channel.mixer.pan = vec![-1.0];
    channel.playlist_mut().unwrap().clips.push(Clip::audio(clip, "Source", source, 0, 0, 512));
    (session, track)
}

#[test]
fn insert_automation_visits_an_internal_breakpoint_in_every_block_size() {
    let (mut session, track) = source();
    let track = session.track_mut(track).unwrap();
    let mut insert = Insert::new("gain");
    insert.params.insert("gain".into(), 0.0);
    track.mixer.inserts[0] = Some(insert);
    let lane = track.lane_mut(&AutoParam::Plugin { slot: 0, param: "gain".into() });
    lane.set_point(0, 0.0);
    lane.set_point(127, -12.0);
    lane.set_point(128, 0.0);
    let expected = render_range(&session, Range::new(0, 512), 16);
    for block in [37, 128, 512] {
        assert_eq!(render_range(&session, Range::new(0, 512), block), expected);
    }
    assert!(expected[0][127] < expected[0][128]);
}

#[test]
fn solo_survives_multiple_aux_returns_in_both_directions() {
    let (mut session, source) = source();
    let a = session.add_bus("A", ChannelFormat::Mono);
    let b = session.add_bus("B", ChannelFormat::Mono);
    session.track_mut(source).unwrap().mixer.output = Route::Bus(a);
    let first = session.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
    let last = session.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
    session.track_mut(first).unwrap().mixer.input = Route::Bus(a);
    session.track_mut(first).unwrap().mixer.output = Route::Bus(b);
    session.track_mut(last).unwrap().mixer.input = Route::Bus(b);
    for solo in [source, last] {
        for track in &mut session.tracks { track.mixer.solo = track.id == solo; }
        let audio = render_range(&session, Range::new(0, 512), 128);
        assert!(audio[0][100] > 0.0);
    }
}

#[test]
fn a_cycle_and_its_dependents_are_reported() {
    let (mut session, _) = source();
    let bus = session.add_bus("Cycle", ChannelFormat::Mono);
    let cycle = session.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
    let dependent = session.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
    session.track_mut(cycle).unwrap().mixer.input = Route::Bus(bus);
    session.track_mut(cycle).unwrap().mixer.output = Route::Bus(bus);
    session.track_mut(dependent).unwrap().mixer.input = Route::Bus(bus);
    let issues = routing_diagnostics(&session);
    assert!(issues.iter().any(|(track, _)| *track == cycle));
    assert!(issues.iter().any(|(track, _)| *track == dependent));
}
