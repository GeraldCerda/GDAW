//! Transport commands. They update the session's stopped playhead and queue requests for the
//! host's audio engine (which may be absent when headless).

use super::*;
use crate::{TransportRequest, cmd};
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "transport.play", "Play", [], Some("Space"), "{from?: position}", always, |e, p| {
            if let Some(at) = position_param(e, "transport.play", p, "from")? {
                e.transport_requests.push(TransportRequest::Locate(at.max(0)));
            }
            e.transport_requests.push(TransportRequest::Play);
            Ok(json!({}))
        }),
        cmd!(noundo "transport.stop", "Stop", [], Some("Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::Stop); Ok(json!({})) }),
        cmd!(noundo "transport.toggle", "Play/Stop", [], Some("Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::TogglePlay); Ok(json!({})) }),
        cmd!(noundo "transport.record", "Arrangement Record", [], Some("Cmd+Space"), "{start_playback?: true}", always, |e, p| {
            e.transport_requests.push(if bool_or(p, "start_playback", true) { TransportRequest::Record } else { TransportRequest::RecordArm });
            Ok(json!({}))
        }),
        cmd!(noundo "transport.loop", "Arrangement Loop", [], Some("Cmd+L"), "{start?, end?, length?, enabled?, selection?: bool}", always, set_loop),
        cmd!(noundo "transport.pause", "Pause", [], None, "{}", always, |e, _| { e.transport_requests.push(TransportRequest::Pause); Ok(json!({})) }),
        cmd!(noundo "transport.half_speed", "Half-Speed Playback", [], Some("Shift+Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::HalfSpeed); Ok(json!({})) }),
        cmd!(noundo "transport.play_selection", "Play Edit", ["Edit", "Selection"], Some("Alt+["), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::PlaySelection); Ok(json!({})) }),
        cmd!(noundo "transport.rtz", "Return to Zero", [], Some("Home"), "{}", always, |e, _| locate(e, 0)),
        cmd!(noundo "transport.go_to_end", "Go to End", [], Some("End"), "{}", always, |e, _| { let end = e.session().content_end(); locate(e, end) }),
        cmd!(noundo "transport.rewind", "Rewind", [], None, "{seconds?: 1}", always, |e, p| {
            let s = e.session();
            let d = s.sample_rate.samples(f64_or(p, "seconds", 1.0).clamp(0.0, 86_400.0));
            let at = (cur(e) - d).max(0);
            locate(e, at)
        }),
        cmd!(noundo "transport.fast_forward", "Fast Forward", [], None, "{seconds?: 1}", always, |e, p| {
            let s = e.session();
            let d = s.sample_rate.samples(f64_or(p, "seconds", 1.0).clamp(0.0, 86_400.0));
            let at = cur(e) + d;
            locate(e, at)
        }),
        cmd!(noundo "transport.locate", "Locate", [], None, "{at: position}", always, |e, p| {
            let at = position_param(e, "transport.locate", p, "at")?.ok_or_else(|| bad("transport.locate", "`at` required"))?;
            locate(e, at.max(0))
        }),
        cmd!(noundo "transport.status", "Transport Status", [], None, "{}", always, |e, _| Ok(json!(e.transport))),
    ]
}

pub(super) fn set_loop(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "transport.loop";
    if e.transport.recording { return Err(bad(id, "Finish this recording before changing the loop brace")); }
    let edit = &e.session().edit;
    let selected = bool_or(p, "selection", false);
    let mut range = edit.arrangement_loop.unwrap_or_else(|| {
        if !edit.selection.is_empty() { return edit.selection; }
        let s = e.session();
        let first = s.tempo.bar_beat_at(edit.selection.start, s.sample_rate);
        let end = s.tempo.samples_at_bar_beat(soundcraft_time::BarBeat { bar: first.bar.saturating_add(4), beat: 1, tick: 0 }, s.sample_rate);
        soundcraft_time::Range::new(edit.selection.start, end.max(edit.selection.start.saturating_add(1)))
    });
    if selected {
        if edit.selection.is_empty() { return Err(bad(id, "Select a time range to loop")); }
        range = edit.selection;
    }
    let start = position_param(e, id, p, "start")?.unwrap_or(range.start);
    let length = position_param(e, id, p, "length")?;
    let end = position_param(e, id, p, "end")?.unwrap_or_else(|| start.saturating_add(length.unwrap_or(range.len())));
    if start < 0 || end <= start { return Err(bad(id, "Loop start must be nonnegative and length must be positive")); }
    let geometry = selected || p.get("start").is_some() || p.get("end").is_some() || length.is_some();
    let enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or(if geometry { true } else { !edit.loop_playback });
    range = soundcraft_time::Range::new(start, end);
    let edit = &mut e.session_mut().edit;
    edit.arrangement_loop = Some(range);
    edit.loop_playback = enabled;
    edit.loop_record = enabled;
    e.transport_requests.push(TransportRequest::Loop(enabled.then_some(range)));
    Ok(json!({"start": start, "end": end, "length": range.len(), "enabled": enabled}))
}

fn cur(e: &Engine) -> i64 {
    if e.transport.playing { e.transport.position } else { e.session().edit.selection.start }
}

fn locate(e: &mut Engine, at: i64) -> Result<Value> {
    let s = e.session_mut();
    s.edit.playhead = at;
    if s.edit.selection.is_empty() || !s.edit.selection.contains(at) {
        s.edit.selection = soundcraft_time::Range::point(at);
    }
    e.transport_requests.push(TransportRequest::Locate(at));
    Ok(json!({"at": at}))
}
