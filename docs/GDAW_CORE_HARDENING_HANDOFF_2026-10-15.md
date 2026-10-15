# GDAW core hardening checkpoint — 2026-10-15

This is an uncompiled source checkpoint, not completion of the exhaustive architecture audit and not a production-readiness certification. Work stopped at the user's request to preserve usage credits.

## Repository and working rules

Work only in GeraldCerda/GDAW, branch main. Fetch the latest head before editing and inspect this commit's diff first. Do not clone, build, compile, generate installers, or run regression suites unless the user explicitly authorizes them. Commit completed source changes to main and independently fetch them to verify. Newly written code must contain no comments. Preserve existing features and avoid unrelated redesigns.

The tester script's source pin remains e1d692457d1ae0050e3f5fb3e9fda1472552cf54. It does not include this checkpoint. Do not advertise a new tester or run its verification gates. The user's existing kit folder is $HOME/Downloads/GDAW_Test_Kit.

## Implemented source changes, not runtime verified

- playback/src/record.rs: bounded preallocated atomic SPSC monitoring ring, whole-frame overflow handling, stale-data discard, single-frame same-rate delivery, capture/stop handshake, take epochs, and flushing final recording-overrun silence on the writer thread.
- playback/src/lib.rs: preallocated atomic meter publication replacing the callback meter mutex and map cloning; bounded UI snapshot retries; initial mixer synchronization before callback ownership; saturating transport advance. Meter publication supports up to 4096 tracks and the existing 16 channels per track.
- mix/src/lib.rs: avoid Rayon dispatch on the marked audio thread; topologically order aux dependencies; propagate cumulative routing latency and independently compensate outputs and sends; include instrument latency; retain meter buffers and reset stale peaks; preserve unity mono-to-mono routing; apply master fader before master inserts.
- mix/src/lib.rs: evaluate fader, trim, mute, VCA, pan, send level/mute/pan, and clip fade/gain automation per sample, including internal lane breakpoints.
- model/src/automation.rs: respect enabled lanes, default missing serialized enabled fields to true, and use overflow-safe high-precision interpolation.
- time/src/tempo.rs and timecode.rs: safer negative-time/bar arithmetic, symmetric negative frame rounding, saturating conversions, and wider drop-frame arithmetic.
- engine/src/cmd/audiosuite.rs: render the isolated selected clip, avoid baking retained editable processing twice, reset rendered stretch, bound metadata, and replace the original clip in place without placement logic splitting neighbors. Original media remains available.
- mix/tests/routing_hardening.rs and inline test modules: added regression coverage for routing, latency, automation, master insert order, floating-point headroom, input ring behavior, recording tails, time edges, and AudioSuite replacement.

No Rust compilation, type/borrow checking, regression execution, hardware recording, hosted-plugin validation, or macOS testing was performed. A Rust syntax parser was used during editing; that does not establish correctness. Review and test this checkpoint before distributing a tester.

## Critical remaining architecture work

1. The callback is NOT allocation-free or fully lock-free. Session command draining/adoption/synchronization, standard channel operations, object reclamation, hosted-plugin paths, and DelayLine resizing remain to be audited and replaced with prepared-state handoff and off-thread reclamation.
2. Delay changes can still allocate/reset buffers and cause discontinuities. Design bounded prepared delay storage and transition handling. Existing per-delay MAX_LATENCY limits can truncate larger cumulative differences. CLAP/AU latency refresh and VST3 notification timing need review.
3. Aux cycles are silenced by topology omission, including their dependents; add explicit routing validation/diagnostics. Transitive aux solo and per-bus master processing remain incomplete. Current master processing assumes a serial main chain.
4. Validate input/output clock alignment, recording offsets, variable callback sizes, underflow/overflow recovery, and drift across separate device clocks. Source playback still needs a bounded disk-streaming architecture review.
5. Plugin automation remains block-start based. Implement sample-offset hosted events or appropriate sub-block processing. Touch/Latch writing remains UI-driven and predominantly fader-oriented; defined Touch return ramps and complete parameter gesture semantics remain unfinished.
6. Validate automation timing through compensated paths, plugin failure transitions, delay bypass/add/remove transitions, and worst-case CPU cost. Automated send matrices are currently evaluated per sample.
7. AudioSuite output is still in-memory until session save, not transactional file-backed processing. Selection-fragment rendering, automatic crossfaded replacements, crash recovery, and full undo/media ownership need review. No alternate-playlist migration bug was established; current find_clip searches the active playlist.
8. Offline rendering already uses MixEngine without hardware clocking; verify matching plugin states, automation, tails, latency trimming, and chunk independence. No claim of full parity has been validated.
9. Finish the requested issue-by-issue audit with file/context, severity, flaw, and exact before/after snippets. Do not label untested replacements production-ready.

## Separate pending user interface request

The user wants instruments presented as ordinary Pro Tools insert slots, with the bottom device interface and no separate tiny instrument add/remove blocks. Track.instrument / INSTRUMENT_SLOT remain separate internally; this request is not implemented by this architecture checkpoint. Prior Pro Tools/Ableton research must not be presented as an implementation.
