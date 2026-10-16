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
| edit_io_readout | `the_io_columns_vol_and_pan_readout_is_readable_in_both_themes` | The painter inspection must respect shape clipping when choosing visible labels and their backing rectangles. Keep the 4.5:1 threshold in both themes and include paint colors/coordinates in any failure. Verification must establish whether this fully explains the reported 1.22:1 result. |
| gdaw_devices | `browser_effects_append_to_the_selected_track_and_can_be_undone` | The starter instrument occupies insert A, so automatic effects append to B and C. Assert this placement, instrument preservation, remaining effect preservation and undo. |
| gdaw_devices | `plugin_browser_exposes_developer_effect_and_format_choices` | Give the format selector an explicit accessible Format label. Check that selector rather than depending on the combo's selected-value text being a separate accessibility node. |
| menu_search | `an_instrument_menu_searches_as_soon_as_it_opens` | Open the ordinary occupied Insert A context menu and type there. Keep keyboard selection and instrument replacement assertions. |
| midi_double_click | `the_smart_tool_opens_it_from_the_lower_half_and_selects_from_the_upper_half` | Selecting the MIDI track can change bottom-device layout. Obtain current clip geometry before the second interaction. Keep upper-half selection and lower-half editor-opening assertions. |
| midi_panel | `midi_panel_resizes_with_a_selected_clip`; `midi_panel_resizes_without_a_selected_clip` | With the browser/device rack present, a 1,000-point window cannot provide the old requested 440-point dock plus the canvas minimum. Isolate the resize contract with those optional panels hidden; retain minimum height, timeline separation, persistence and session-preservation checks. |
| midi_panel | `layout_reset_cancels_a_held_midi_divider_without_editing_the_session`; `arrange_cancels_a_held_midi_divider_without_editing_the_session`; `legacy_visibility_sync_cancels_a_held_midi_divider_without_editing_the_session` | Use the same sufficient-space fixture. Preserve held-drag cancellation, layout serialization and complete session equality checks. |
| midi_panel | `only_pencil_inserts_notes_in_the_midi_roll` | Independent tool clicks at the same coordinate were close enough to become the editor's intentional double-click-to-add gesture. Separate them in time; retain unchanged-session assertions for every other tool, plus pencil insertion and undo. |
| mix_instrument_contrast | `populated_mix_instrument_uses_readable_palette_in_real_frames` | Placing an effect explicitly in occupied insert A replaces the instrument by design. Put the comparison effect in B, show the insert bank, and replace the instrument through its context menu. Retain all four theme cases, hover checks, 4.5:1 contrast threshold, audio-pool preservation, defaults and undo. |
| mix_narrow_headers | `normal_strips_keep_the_full_section_names`; `narrow_strips_use_short_section_names_that_fit` | Remove expectations for the deleted INSTRUMENT/INST section header; retain insert/send labels, width and non-elision checks. |
| mix_separate | `mix_in_a_separate_window_toggles_and_closing_it_returns_to_the_main_window` | The current toggle hides/reopens the separate mixer while Edit remains. Explicitly test both transitions before drawing and closing it. |
| track_list | `dragging_a_track_list_row_moves_the_track_to_the_gap_it_is_dropped_in`; `clicking_a_track_list_row_still_selects_the_track` | The browser moves the track list beyond the old global x < 150 assumption. Locate the leftmost painted matching track name; preserve exact reordered track sequences and selection checks. |
| tracks_scroll_bar | `the_tracks_gutter_pages_a_long_list` | The default theme is Light; the test searched only for the Dark palette thumb. Use the fixture's actual palette and retain the required page movement. |

The unused `Mark::Dot` branch is removed instead of suppressing its warning. No failing test is ignored or disabled. No tolerance or contrast requirement is reduced.

## Verification boundary

Commit `7dc6ca978567e67791e3510fcca6c4b5fc327c80` contains the initial corrections and a macOS GitHub Actions workflow. Its 19 changed files were independently fetched at that commit and compared byte for byte after updating main. The workflow fetches the exact source archive, without cloning, runs the six core/UI package suites with the locked release configuration, and builds the desktop binary. Further failures must be addressed before publishing a replacement tester.

Follow-up commit `34d1351214cfda086017d23ab4ed962c2ebfd763` corrects the instrument replacement interaction, adds improved contrast failure diagnostics and guards against the removed instrument header returning. It also fixes an invalid egui combo-box API call caught by the first CI attempt, and packaging shell lint failures (missing interpreter declarations and static analysis of the machine-local Cargo environment file). All eight follow-up files were independently fetched and compared byte for byte.

Packaging lint passed: https://github.com/GeraldCerda/GDAW/actions/runs/38849593554

The Apple Silicon macOS release regression step is still running: https://github.com/GeraldCerda/GDAW/actions/runs/38849593338 . The desktop binary build is queued after it. No clean compilation/test result is claimed for this revision yet. This document must be updated after that run and any necessary corrections. The complete original log contains 796 passing and 23 failing tests, with none ignored.

Passing these gates will establish the tested build and regression results, not production readiness. Hardware recording/playback, independent-device drift, callback deadlines/allocations/locks, third-party plugin lifecycle/automation, audible delay transitions and AudioSuite crash recovery are not established by these tests. The remaining architecture work in `GDAW_CORE_CONTINUATION_2026-10-15.md` remains open. The tester source pin is unchanged.
