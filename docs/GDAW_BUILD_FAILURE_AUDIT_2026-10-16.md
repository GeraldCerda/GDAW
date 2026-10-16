# Build failure audit — 2026-10-16

Input: the complete 5,887-line `build-20261015-202624.log`, source revision `716fe4be7737e9efbfe5c7ef03a3a5d724c946c9`, and main revision `b14fa4fee5ff38dbbe5b42a730c85c58fb425fd3` before edits.

The release test executables compiled successfully in that log. Execution then reported 23 failing tests in 15 targets. The test-kit driver exited at its regression gate, before its desktop binary build and application installation. The unused `Mark::Dot` variant was the only compiler warning reported. This audit does not equate test executable compilation with a successful desktop build.

## Complete failure ledger

| Target | Failed tests | Cause and correction under verification |
| --- | --- | --- |
| mix library | `surround_busses_and_master_fader` | The mono source-to-mono bus path now preserves unity. The old expected value applied an extra center-pan attenuation. Check unity on the left and silence on the right after the hard-left aux. Keep the surrounding multichannel master-fader checks. |
| routing_hardening | `fader_and_pan_breakpoints_are_independent_of_render_block_size` | The documented pan law is exactly −3 dB, while the assertion used the −3.0103 dB square-root constant. Use the exact decibel gain; retain every-sample comparisons across block sizes and both hard-pan endpoint checks. |
| UI library | `every_enabled_section_reserves_space_above_the_fader` | Height expectations included the removed 38-point instrument row. Reserve space for the actual insert/send/I/O sections: 670 points by default and 1,034 for all sections. |
| bounce | `a_bounce_renders_across_frames_with_progress_until_its_file_is_written` | A global-idle harness wait cannot finish while the interface schedules recurring repainting. Step until the bounce itself finishes, with a bounded limit, then retain progress, decoded frame-count, status and temporary-file checks. Avoid continuous device-chain polling when there is no player. |
| command_reachability | `every_command_has_a_ui_path` | Routing diagnostics had no UI path. Add Setup > Routing Diagnostics and show returned track names/problems in the status line. Keep the reachability assertion and its existing gap list unchanged. |
| edit_io_readout | `the_io_columns_vol_and_pan_readout_is_readable_in_both_themes` | The painter inspection must respect shape clipping when choosing visible labels and their backing rectangles. Keep the 4.5:1 threshold in both themes and include paint colors/coordinates in any failure. Both themes now pass the unchanged 4.5:1 check on visible labels. |
| gdaw_devices | `browser_effects_append_to_the_selected_track_and_can_be_undone` | The starter instrument occupies insert A, so automatic effects append to B and C. Assert this placement, instrument preservation, remaining effect preservation and undo. |
| gdaw_devices | `plugin_browser_exposes_developer_effect_and_format_choices` | Give the format selector an explicit accessible Format label. Check that selector rather than depending on the combo's selected-value text being a separate accessibility node. |
| menu_search | `an_instrument_menu_searches_as_soon_as_it_opens` | Open the ordinary occupied Insert A context menu and type there. Keep keyboard selection and instrument replacement assertions. |
| midi_double_click | `the_smart_tool_opens_it_from_the_lower_half_and_selects_from_the_upper_half` | Selecting the MIDI track can change bottom-device layout. Settle the device-panel layout, scroll the full clip into the visible timeline, and assert that each click lies inside the current viewport. Recomputing geometry alone did not fix the remaining failure. Keep upper-half selection and lower-half editor-opening assertions; both now pass with the browser and device panel enabled. |
| midi_panel | `midi_panel_resizes_with_a_selected_clip`; `midi_panel_resizes_without_a_selected_clip` | With the browser/device rack present, a 1,000-point window cannot provide the old requested 440-point dock plus the canvas minimum. Isolate the resize contract with those optional panels hidden; retain minimum height, timeline separation, persistence and session-preservation checks. |
| midi_panel | `layout_reset_cancels_a_held_midi_divider_without_editing_the_session`; `arrange_cancels_a_held_midi_divider_without_editing_the_session`; `legacy_visibility_sync_cancels_a_held_midi_divider_without_editing_the_session` | Use the same sufficient-space fixture. Preserve held-drag cancellation, layout serialization and complete session equality checks. |
| midi_panel | `only_pencil_inserts_notes_in_the_midi_roll` | Independent tool clicks at the same coordinate were close enough to become the editor's intentional double-click-to-add gesture. Separate them in time; retain unchanged-session assertions for every other tool, plus pencil insertion and undo. |
| mix_instrument_contrast | `populated_mix_instrument_uses_readable_palette_in_real_frames` | Placing an effect explicitly in occupied insert A replaces the instrument by design. Put the comparison effect in B, show the insert bank, and replace the instrument through its context menu. Retain all four theme cases, hover checks, 4.5:1 contrast threshold, audio-pool preservation, defaults and undo. |
| mix_narrow_headers | `normal_strips_keep_the_full_section_names`; `narrow_strips_use_short_section_names_that_fit` | Remove expectations for the deleted INSTRUMENT/INST section header; retain insert/send labels, width and non-elision checks. |
| mix_separate | `mix_in_a_separate_window_toggles_and_closing_it_returns_to_the_main_window` | The current toggle hides/reopens the separate mixer while Edit remains. Explicitly test both transitions before drawing and closing it. |
| track_list | `dragging_a_track_list_row_moves_the_track_to_the_gap_it_is_dropped_in`; `clicking_a_track_list_row_still_selects_the_track` | The browser moves the track list beyond the old global x < 150 assumption. Locate the leftmost painted matching track name; preserve exact reordered track sequences and selection checks. |
| tracks_scroll_bar | `the_tracks_gutter_pages_a_long_list` | The default theme is Light; the test searched only for the Dark palette thumb. Use the fixture's actual palette and retain the required page movement. |

The unused `Mark::Dot` branch is removed instead of suppressing its warning. No failing test is ignored or disabled. No tolerance or contrast requirement is reduced.

## Verified result

Verified source revision: `964ecaa999ad8feb17bce77c3830005ba1e4b656`.

- Apple Silicon native desktop build: https://github.com/GeraldCerda/GDAW/actions/runs/38855525806
- Complete core/UI regressions: https://github.com/GeraldCerda/GDAW/actions/runs/38855525758
- Packaging lint: https://github.com/GeraldCerda/GDAW/actions/runs/38854517695

The desktop command was `cargo +stable build --locked --release --target aarch64-apple-darwin -p soundcraft`, with `MACOSX_DEPLOYMENT_TARGET=11.0`. Rust 1.99.1 produced a Mach-O arm64 executable. The build completed in 2m 17s with zero compiler errors and zero warnings. The executable existed and was executable; it was not launched.

The regression command was `cargo +stable test --no-fail-fast --locked --release -p soundcraft-dsp -p soundcraft-model -p soundcraft-engine -p soundcraft-mix -p soundcraft-playback -p soundcraft-ui-egui`, with `CARGO_PROFILE_RELEASE_LTO=off` for test executables only. All 819 tests passed, none failed and none were ignored, across 87 result targets. Compilation completed in 3m 28s, with zero compiler warnings or errors. All 23 failures from the original log now pass.

The original full-LTO test compilation was cancelled before tests ran because compiling dozens of separate optimized test executables was taking excessively long. No successful result is claimed for that run. Disabling test-only link-time optimization preserves every test and assertion. The independently verified desktop application uses its normal release link settings. The tester's optional local verification command matches the tested CI configuration.

The preceding complete run had 818 passing tests and one remaining Smart Tool MIDI failure. Its fixture still used a point on a row partly outside the timeline after device-panel reflow. The final fixture reveals the complete clip and checks click visibility before exercising the same selection/opening behavior. It retains the new browser/device defaults.

The first CI attempt also caught an invalid egui combo-box API call in this fix set. It was corrected to the constructor supported by egui 0.36.2. Packaging lint caught missing shell interpreter declarations and static analysis of a machine-local Cargo environment file; the entry points and Cargo PATH setup were corrected. These intermediate failures are not omitted from the verification history.

All 24 changed files between the prior main checkpoint and the verified source revision were independently fetched at that revision and compared byte for byte. Subsequent tester/report changes pin that verified source; they do not change application source.

## Exactly what remains untested

- The built application was not launched. A signed app bundle, installation, launcher execution, microphone permissions, interactive GUI use and actual audio operation on the user's Mac were not runtime-tested by CI. The packaging scripts passed static lint; that does not establish installation success.
- Physical playback/capture, Apollo routing, independent input/output clocks, recording offsets, variable device callback sizes, underflow/overflow recovery, long recording tails, real-time allocation/lock/reclamation behavior, thread races, worst-case callback deadlines and CPU/memory stress have not been established. Automated mixer/ring/mailbox tests do not substitute for those measurements.
- Real third-party CLAP, VST3 and Audio Unit plugins, native editors, state adoption/reclamation, dynamic latency changes, failure transitions, native sample-offset automation and complete Touch/Latch parameter gestures have not been validated against installed plugins.
- Audible delay transitions and bypass/add/remove continuity, long-chain phase/null behavior, hardware-clock timing, realtime/offline parity, plugin tails and extreme chunk-size combinations remain unvalidated beyond the particular automated cases that passed.
- AudioSuite crash/failure injection, automatic recovery discovery/replay, unsaved original-media durability, media garbage collection/ownership and partial-selection/crossfaded replacements have not been validated or completed.
- This run executed the six packages listed above. Dependency crates compiled, but their separate unit suites outside that list were not separately run. Whole-workspace regression/xtask CI, clippy, formatting gates, asset/layer checks and WASM checks were not run here. The fixed revision's default full-LTO test-executable configuration was not completed; the full regression suite passed with test-only LTO disabled.
- Intel macOS, Windows, Linux, FreeBSD and web builds/runtimes were not verified in this pass. macOS installation/signing/notarization or distributable-installer validation was not performed.

## Remaining implementation

The open architecture items in `GDAW_CORE_CONTINUATION_2026-10-15.md` remain open: generalized parameter gesture state; native hosted automation events; timestamp-driven capture alignment; the remaining callback/plugin lifecycle and bounded streaming audit; prepared-delay capacity replenishment; automatic AudioSuite recovery and media ownership plus partial-selection/crossfaded replacements; and instrument DSP ordering/overflow presentation.

The build and regression failures recorded in this log are resolved for the verified configuration. This is not a claim that the architecture is complete or production-ready.
