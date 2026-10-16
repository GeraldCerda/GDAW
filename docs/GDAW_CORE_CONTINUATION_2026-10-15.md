GDAW core continuation — 2026-10-15
===================================

Base: 25d09ab3caf5ed5508a902276deedf816e9841c9, GeraldCerda/GDAW main.

Status: source continuation, incomplete architecture hardening. This is neither a production-readiness claim nor a compiled or runtime-tested release. Selected repository files were fetched through GitHub; no repository was cloned. No compiler, application, regression suite, installer, tester, or workflow was run.

Source changes
--------------

1. Critical — playback state adoption and reclamation
Context: crates/playback/src/lib.rs, mailbox.rs, Cargo.toml, Cargo.lock.
Before: Cmd::Session adopted plugins, synchronized the mixer, resized output/input buffers and sent retired objects using standard channels inside the callback.
Before:
```rust
self.mix.adopt(instances);
self.mix.ensure_synced(&self.session);
self.pending = vec![vec![0.0; BLOCK]; nch];
```
After: PreparedPlayback creates/synchronizes a replacement mixer and allocates its storage on the control thread. The callback inherits compatible live instances, swaps state and retains the old state until a bounded retirement mailbox accepts ownership. PreparedInput similarly carries monitoring storage. Only one session replacement may be in flight; additional updates coalesce on the control thread. The command drain has a 64-command limit. The bounded concurrent-queue dependency already existed in Cargo.lock and is now a direct playback dependency.
After:
```rust
ready.mix.inherit(&mut self.mix, &ready.session, &self.session, &ready.fresh);
std::mem::swap(&mut self.mix, &mut ready.mix);
std::mem::swap(&mut self.session, &mut ready.session);
self.retired = Some(ready);
```
A full retirement queue retains its payload and defers adoption. A full control queue marks the output fault rather than silently continuing with a lost transport command. Legacy Cmd::Session/Input paths remain cfg(test) only; old tests do not establish coverage of the new production handoff.

2. Critical — delay storage and transitions
Context: crates/mix/src/lib.rs and pad_fx.rs.
Before:
```rust
self.idx = 0;
self.bufs = vec![vec![0.0; len]; ch];
```
After: synchronization prepares delay storage off the callback under a shared storage budget. DelayLine::set does not allocate. Zero-delay paths continue recording history. Delay changes crossfade between two integer taps over 128 frames; a change during a transition is queued. Matching prepared histories are inherited. A request beyond prepared capacity silences that delayed path and is exposed through MixEngine::delay_capacity_exceeded().
After:
```rust
let a = ring.get((self.idx + size - previous) % size).copied().unwrap_or(0.0);
let b = ring.get((self.idx + size - self.len.min(size - 1)) % size).copied().unwrap_or(a);
*sample = a + (b - a) * blend;
```
This does not prove arbitrary delay growth, plugin add/remove, failure, or bypass changes are glitch-free. History inheritance currently requires compatible widths and sufficient old storage. Automatic off-thread capacity expansion and user-visible diagnostics still need integration.

3. High — render preparation
Context: MixEngine::sync, ensure_synced, render, structure_fingerprint.
Before: render could synchronize changed structure on a marked realtime thread.
After:
```rust
if on_audio_thread() { self.preparation_required = true; return; }
```
An unprepared marked render outputs silence. Fingerprinting now includes parameter values, send slot positions, clip identities and edit values needed to prepare caches. Clip effects and parameter caches are warmed during synchronization. Strip work uses existing map entries instead of removing/reinserting keys, avoiding tombstone-driven rehash growth.
Remaining: end-to-end allocation/lock instrumentation, plugin reset/set_param/process/latency/editor bridges, large-session fingerprint cost, final stream shutdown ownership, and bounded disk streaming are not completed.

4. High — plugin automation
Context: apply_params, process_insert and instrument rendering.
Before:
```rust
apply_params(slot, ins, t, i, pos);
slot.plugin.process(&mut strip.buf, frames);
```
After: automated insert paths evaluate every timeline sample and process preallocated one-sample buffers. Instrument automation uses matching one-sample MIDI scheduling and transport updates. Lanes absent from the static parameter map are also applied. Nonautomated paths retain block processing.
After:
```rust
apply_params(slot, insert, track, address, at);
slot.plugin.process(&mut slot.sample, 1);
```
This is a fallback rather than native CLAP/VST3/AU sample-offset event delivery. Host parameter setters may still lock or allocate. Plugin block-size assumptions, controller ordering, instrument cost, latency notifications, compensated-path automation and offline/realtime parity remain unvalidated. Worst-case CPU cost may be substantial.

5. High — routing
Context: routing_diagnostics, solo_audibility, process_bus_masters, engine command rollback.
Before: solo inheritance only examined direct feeders; every master processed the main mix.
After: graph traversal includes ancestors and descendants of soloed tracks. Cycles, blocked dependents and missing buses are reported by mix.routing_diagnostics. Document edits introducing new routing issues are rejected and rolled back; session/file loading can still open damaged documents for repair. Bus-targeted masters process their assigned bus once, before its aux consumers, using that bus's width; their latency contributes to downstream and stem latency.
After:
```rust
if let Route::Bus(bus) = &t.mixer.input { self.process_bus_masters(s, *bus, pos, frames); }
```
Hardware output-path coverage, automation through compensated branches, muted-send dependency policy and complex graph null tests remain outstanding.

6. High — monitoring clocks
Context: InputRing::align_clocks/pop_into and Player::set_input.
Before: monitoring consumed input using only nominal sample-rate ratios.
After: optional occupancy-based, smoothed correction is bounded to 2000 ppm; underflow frames are counted and an empty-ring underflow resets stale fractional phase. Player enables the correction for its input ring.
This is not timestamp-based duplex alignment. It does not measure converter latency or correct recorded takes against the output device clock. Startup priming, real device timestamps, drift outside the bound, variable-size callbacks, recovery sound quality and recording offsets remain unfinished.

7. High — Touch release
Context: ui-egui/src/lib.rs, write_automation.
Before: Touch stopped adding points immediately on release; TouchLatch faders latched.
After: the original fader lane is captured at touch start and release writes a 100 ms return ramp to that lane. TouchLatch faders use Touch behavior; Latch retains its existing write-until-stop behavior.
This remains a UI-timed fader implementation. Complete pan/mute/send/plugin/controller gesture begin/update/end handling, per-parameter overrides during a gesture, sample-clock write scheduling, brief gestures between UI ticks and generalized TouchLatch semantics remain unfinished.

8. Critical — AudioSuite publication and media
Context: engine/src/cmd/audiosuite.rs and io.rs.
Before:
```rust
let s = e.session_mut();
let src = crate::io::add_source(s, &new_name, buf, None, FileFormat::Wav);
```
After: rendering and clip replacement occur in a staged session. All new media is validated, encoded as float WAV, flushed and renamed before the staged session is published. A private transaction directory includes before.scraft and ready.scraft recovery snapshots. An incomplete transaction removes only its own directory; completed media remains available to undo snapshots. Unsaved sessions use a temporary media root.
After:
```rust
crate::io::persist_processed(e.session(), &mut staged, &media)?;
if done > 0 { *e.session_mut() = staged; }
```
No automatic recovery discovery/replay, media garbage collector or crash-injection verification exists yet. Previously unsaved original sources are not made durable by this transaction. Partial-selection fragment processing and replacement crossfades remain unfinished. Temporary unsaved-session media is not a durable user-selected project location.

9. Medium — instruments in insert cells
Context: model/src/track.rs; engine mix/instrument commands; Edit, Mix and bottom device UI.
Before: a separate instrument row and separate instrument picker block.
After: instruments occupy ordinary lettered insert cells; insert menus/search/drop support instrument selection; bypass/removal and parameters route to the instrument; the bottom device chain follows displayed slot order. The internal INSTRUMENT_SLOT host address remains intact. The serialized instrument_slot is optional, preserving old sessions' lack of instrument insert automation addressing.
Remaining: instrument DSP still precedes the effect chain internally, even if its displayed slot is later. Moving an instrument arbitrarily through the DSP chain is not implemented. Legacy sessions with an instrument plus all ten effect slots retain the instrument as overflow K in the bottom device chain; Edit/Mix overflow presentation needs completion. Legacy instrument automation requires explicit slot assignment by reselecting its instrument. No visual render or interaction validation was performed.

Verification and continuation
-----------------------------

New test source covers a steady-signal delay transition, mailbox ownership on saturation, insert automation across block sizes, transitive aux solo and routing-cycle diagnostics. These tests have NOT been executed. Existing tests were not executed either.

Verification performed: inspect the checkpoint diff and relevant source, inspect pinned concurrent-queue API source, review changed ownership/routing paths, textual delimiter checks on core files, inspect workflow triggers, and verify committed files and main independently through GitHub after publication. Delimiter checks are not Rust parsing or type checking.

Still untested: all Rust syntax/type/borrow/lint/link/build checks; every new and existing test; callback allocation/lock behavior and deadlines; race and saturation behavior; hardware capture/playback, converter latency and device clocks; hosted CLAP/VST3/AU behavior and native event timing; PDC transitions and phase/null parity; offline parity, tails and chunk independence; UI layout and interaction; AudioSuite failure/crash/reopen/undo/media ownership; macOS, Windows and Linux behavior.

Next source work: full parameter gesture state machine; native hosted automation event APIs; timestamp-driven capture/output alignment and take placement; prepared-delay capacity replenishment with continuity; remaining plugin lifecycle audit and streaming architecture; partial-selection/crossfaded AudioSuite replacements plus automatic recovery and media ownership; finish instrument DSP ordering/overflow presentation. Compilation and execution require the user's authorization.

Tester source pin remains e1d692457d1ae0050e3f5fb3e9fda1472552cf54. No tester packaging or installer file is changed. No new tester is requested or advertised.

Verification update — 2026-10-16 UTC
-----------------------------------

Revision 964ecaa999ad8feb17bce77c3830005ba1e4b656 now passes the complete six-package DSP/model/engine/mix/playback/UI regression run (819 passed, none failed or ignored) and the normal Apple Silicon release desktop build, both with zero compiler warnings/errors. Regression executables use test-only LTO disabled; the desktop application retains its normal release link settings. The historical uncompiled/untested statements above describe preparation of the earlier checkpoint, not the present verification result.

Read [GDAW_BUILD_FAILURE_AUDIT_2026-10-16.md](GDAW_BUILD_FAILURE_AUDIT_2026-10-16.md) for every original failure, the exact commands/results and all unperformed checks. Application launch/installation, hardware and real hosted plugins remain unvalidated. The remaining architecture implementation items above are still incomplete. The tester was updated only after source review, independent GitHub verification and the passing results, following the user's request for another tester.
