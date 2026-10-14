**GDAW defect audit — Busy Works Beats SoundCraft review**

Source video: https://www.youtube.com/watch?v=LceYcJOzFcY
Title: Someone Rebuilt Pro Tools and Made it Open Source?!
Publisher: Busy Works Beats
Published: 2026-10-10
Runtime: 11:23
Audit date: 2026-10-14
Transcript source: YouTube captions retrieved via vidIQ (English, US)
Code examined: GeraldCerda/GDAW main, SoundCraft 0.6.1
Scope: observed usability failures and reviewer concerns, compared against the currently accessible source. No hardware or GUI runtime testing has been performed.

**Interpretation**

This is a paraphrased issue inventory rather than a reproduction of the video's copyrighted transcript. The retrieved transcript contains no per-sentence timestamps, so exact playback offsets are intentionally omitted. Reviewer observations are evidence of the video's test session, not proof that the present fork reproduces every issue. Severity is based on a professional audio recording workflow. Status words refer to code inspection only.

**P0 — Recording and I/O blockers**

1. Physical I/O routing matrix absent.
Reviewer observation: no Pro Tools-like input/output matrix and no practical method to assign specific physical microphone channels.
Code: crates/ui-egui/src/windows.rs, io_setup; docs/gaps.md G-01.
Status: confirmed gap in the current UI. It exposes named busses and outputs, but not an interactive per-channel hardware matrix.
Acceptance: enumerate an interface's inputs and outputs, map physical channels to tracks and hardware returns, preserve mappings across reopen and device changes, and show disconnected/unavailable channels.

2. Echo or doubled input-monitor signal.
Reviewer observation: microphone monitor audibly echoed or doubled during testing.
Code: crates/ui-egui/src/lib.rs ensure_input; crates/playback/src/record.rs InputRing; docs/gaps.md G-01.
Status: reviewer-reported, requires hardware reproduction. The input and output streams are separate; direct interface monitoring can coexist with software input monitoring.
Acceptance: with direct monitoring on and off, record and monitor a mono microphone on a supported audio interface without unintended duplicate monitor paths. Report capture-to-output latency and compensate recorded-take placement.

3. Unclear input armed/live-meter state.
Reviewer observation: no obvious live signal on the input path, leaving the microphone's status uncertain until recording.
Code: crates/ui-egui/src/mix_window.rs meters and arm/input-monitor buttons; crates/ui-egui/src/lib.rs ensure_input.
Status: requires GUI and interface test. General track meters and arm/monitor toggles exist in source, but observable input monitoring before recording has not been verified.
Acceptance: input channel selection, arm state, live pre-fader level, peak/clip indication, and missing-device status are visible and update before record starts.

4. Reliable channel-by-channel recording setup.
Reviewer observation: difficult to identify which microphone is recorded, especially with multiple microphone inputs.
Code: crates/playback/src/record.rs open_with and arm; crates/ui-egui/src/windows.rs io_setup.
Status: physical channel mapping remains incomplete. Device selection does exist in code.
Acceptance: choose a multichannel interface, assign different mono inputs to separate tracks, arm both, record isolated sources, and verify the resulting files are not duplicated or cross-fed.

**P1 — Basic controls, insertion, and editing**

5. Delete bus appears ineffective.
Reviewer observation: the I/O dialog's Delete button did not appear to remove a bus.
Code: crates/ui-egui/src/windows.rs io_setup; crates/engine/src/cmd/setup_more.rs delete_bus.
Status: needs reproduction. Both a Delete UI action and a removal handler are present, so absence of implementation must not be claimed.
Acceptance: create a bus, delete it in the dialog, verify it disappears immediately, validate affected routes/sends reset or are reassigned as specified, and confirm Undo behavior.

6. Recorded audio deletion is difficult to discover.
Reviewer observation: a prominent delete action did not remove the selected recording; reviewer discovered a right-click route.
Code: crates/engine/src/cmd/clip.rs and edit commands; crates/ui-egui/src/edit_window.rs.
Status: workflow discoverability issue requiring a UI regression test.
Acceptance: selecting an audio clip and pressing Delete/Backspace follows one predictable behavior; contextual deletion and source-file deletion are clearly distinguished and undoable.

7. Clip gain and clip volume are difficult to find.
Reviewer observation: could not immediately adjust gain and interpreted the Render Clip Gain menu as the only option.
Code: crates/model/src/clip.rs gain_db/gain_env; crates/engine/src/cmd/clip.rs clip.gain; crates/ui-egui/src/edit_window.rs clip_gain_icon and Gesture::ClipGain.
Status: control exists but its visibility depends on view flags, clip height, and available clip width. This is a discoverability/accessibility deficit, not missing DSP.
Acceptance: selected clips expose clear gain controls at practical track heights, adjustment is nondestructive, gain values are visible, and drag/keyboard operations work.

8. Track volume and pan controls are difficult to locate.
Reviewer observation: struggled to identify simple track volume and left/right pan controls.
Code: crates/ui-egui/src/mix_window.rs fader, pan widgets; crates/engine/src/cmd/mix.rs.
Status: present in code; interface comprehension needs review.
Acceptance: an untrained user can select a track, adjust dB and pan, hear and meter the change, and restore default values without opening hidden menus.

9. Insert/plugin creation is hard to discover.
Reviewer observation: could not figure out how to add an effect through Pro Tools-style insert slots.
Code: crates/ui-egui/src/mix_window.rs insert_slot and plugin menu; crates/engine/src/cmd/mix.rs mix.insert.
Status: insert engine exists. Visibility, hit targets, interaction model, and discovery need validation.
Acceptance: empty inserts expose an understandable add affordance, a categorized searchable plugin browser, drag-drop insertion, bypass/reorder/remove, and recognizable failure messages.

10. Click track and metronome semantics are confusing.
Reviewer observation: creating a click track did not produce the expected audible click.
Code: crates/mix/src/lib.rs metronome mixing; crates/engine/src/cmd/track.rs; session metronome flags.
Status: needs runtime reproduction. Audio metronome processing is implemented; it is not safe to claim the entire feature is missing.
Acceptance: clicks occur at expected beat subdivisions while playing or recording under documented toggles; named click tracks either generate the intended click or clearly communicate their role.

11. Track creation name field is difficult to read.
Reviewer observation: the New Tracks dialog name entry appeared visually broken or too narrow.
Code: crates/ui-egui/src/dialogs.rs Dialog::NewTracks.
Status: UI defect reported by reviewer; needs theme/scale reproduction.
Acceptance: name field text, cursor, selection, and placeholder are legible at common macOS display scales in light and dark modes; long names scroll correctly.

12. General editing and controls feel unintuitive.
Reviewer observation: right-click requirements, slider behavior, unlabelled insert slots, and controls hidden by view modes made simple actions difficult.
Code: crates/ui-egui/src/edit_window.rs; crates/ui-egui/src/mix_window.rs; crates/ui-egui/src/widgets.rs; crates/ui-egui/src/menus.rs.
Status: cross-cutting UX finding rather than a single code bug.
Acceptance: common recording, clip gain, deletion, plugin insert, pan, and volume workflows are discoverable with a consistent click model and visible tooltips/focus states.

**P2 — Effect usability and verification**

13. AudioSuite-like effects offer generic parameter sliders rather than specialized editors.
Reviewer observation: offline channel-strip, compression, EQ, pitch, reverb, and delay interfaces were mostly parameters/sliders, making the effects difficult to understand.
Code: crates/ui-egui/src/ops_windows.rs; crates/ui-egui/src/windows.rs; crates/dsp/src.
Status: effects exist, but editor ergonomics are limited.
Acceptance: built-in effect panels expose clear units, signal flow, parameter groups, sensible defaults, bypass, wet/dry where applicable, input/output levels, and usable numerical entry.

14. Effects are hard to audition and level-match.
Reviewer observation: EQ changes were hard to hear, compression needed manual makeup gain to demonstrate impact, and other processing required additional gain changes.
Code: crates/engine/src/cmd/clip.rs; crates/dsp/src; offline processing controls.
Status: reviewer confirmed audible compressor, reverb, delay, and pitch changes. No proof of a broken effect algorithm from the transcript.
Acceptance: before/after preview and A/B, explicit processing state, undo, and gain-matched listening allow users to verify changes without destructive experimentation.

15. Clip pitch workflow lacks Ableton-level manipulation.
Reviewer observation: offline octave pitch shifting works, but adjustment and subsequent gain compensation were cumbersome.
Code: crates/model/src/clip.rs stretch; crates/engine/src/cmd/clip.rs; docs/gaps.md G-15.
Status: existing DSP and offline pitch shift do not provide requested per-clip real-time pitch/formant envelopes.
Acceptance: non-destructive per-clip transpose in cents/semitones, editable formant control, continuous pitch envelope, warp-marker manipulation, tempo sync, undo, and repeatable export.

**Review criticisms that must not be logged as missing functionality**

- Playback Engine now exposes audio system, input/output devices, sample rate, and buffer-size choice: crates/ui-egui/src/windows.rs playback_engine. This was added after the reviewer published the video. True duplex routing and record latency compensation still require work.
- Recorder implementation now streams each armed take to a WAV file through a writer thread: crates/playback/src/record.rs. Older summary documentation that calls recording RAM-only is stale for this source. Whole-source decoding into memory may still be a separate scaling concern.
- Faders, pan controls, plugin insert commands, clip gain, and bus-delete handlers exist in source. This does not settle their GUI reliability, intuitiveness, or hardware behavior.
- The reviewer actually obtained a recorded vocal and audible compressor, reverb, delay, and pitch effects. The entire application must not be characterized as a nonfunctional shell.

**Related production backlog not demonstrated by this video**

- Record MIDI from an external controller; see docs/gaps.md G-02.
- Add Ableton-style MIDI editing and exact musical-grid snapping; see crates/ui-egui/src/midi_editor.rs.
- Add a multi-pad Drum Rack with separate sample chains and routing; see crates/ui-egui/src/sampler_window.rs.
- Add a persistent searchable left-side effects, instruments, plugins, and sample browser.
- Add high-quality pitch/formant automation and richer warping algorithms; see docs/gaps.md G-15.
- Verify signed macOS third-party plugin support and plugin editor integration; see docs/gaps.md G-05.

**Execution priority**

First secure real recording: physical I/O assignments, reliable low-latency monitoring, clear armed-input meter, and multichannel capture. Then repair the everyday UX workflows in the review. Keep the Pro Tools-inspired mixer and playlists intact. Integrate MIDI grid, Drum Rack, sample browser, and clip-level warp/pitch/formants in isolated, testable changes. Do not merge a change based only on UI screenshots; verify command behavior, session save/reopen, undo/redo, audio bounces, and macOS hardware where relevant.
