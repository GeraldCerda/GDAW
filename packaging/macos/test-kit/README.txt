GDAW Mac Test Kit v4

Source revision: d60c08d7b79187518302218b49e13dbec883607c
Repository: https://github.com/GeraldCerda/GDAW

This kit downloads pinned source from GeraldCerda/GDAW and builds it on your Mac. No DAW binary, installer or source ZIP was built while preparing these changes. Existing v3 folders and Cargo caches remain usable with the updated build_test.sh.

Run the regression suites before installing this pass:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v3/build_test.sh" --verify

Use the actual kit folder if yours is named v4. The verification option checks formatting, then runs the DSP, model, engine, mix, playback and UI tests. A test failure stops before installation. Formatting failures are reported; compilation and regression checks still follow. Running without --verify compiles and installs without running those tests.

The script checks Apple Command Line Tools, installs or updates Rust, downloads this source revision, builds the Mac application and signs it locally. GDAW Test.app goes to ~/Applications. Previous copies are retained as timestamped backups. The existing launch_test.sh keeps settings in ~/Library/Application Support/GDAW Test. Logs remain in the kit's logs folder.

Implemented source changes in this pass:
- Drum Rack has ten labelled effect slots A-J for each pad. Add, remove and bypass effects, adjust parameters or drop an effect/preset onto an exact slot. Pad processing precedes the track inserts. AU, VST3 and CLAP use the existing host, native editors and state capture. Pad chains save with the session and support undo.
- Pad chains align their processing latency. Invalid audio bypasses the failing pad effect and reports its slot while other pads continue.
- Audition follows the selected sample's tuned duration and device rate. Short hits on the same rack preserve earlier long hits. Meters follow the actual preview lifetime.
- Native keypad identity is preserved separately from the number row. Transport-mode keypad 0-9, supported Command/Option keypad windows, keypad grid/nudge changes, period-number-period recall and keypad Enter memory-location entry are connected. Return remains separate.
- Instrument editor parameter changes are stored through the instrument command instead of being discarded.

Existing starter layout remains: MIDI 1 with Drum Rack, mono Audio 1 on physical input 1, visible I/O and insert/send columns, left browser, bottom devices, light appearance, eighth-note Grid and Purple G branding. Cmd+= toggles a separate mixer. Plugin groups and format filters remain available in the browser and insert menus. Drum libraries use the user's assigned folder; no samples are bundled.

Mac checks after the regression suites pass:
1. Compare number-row Command+1 with keypad Command+1. Check keypad transport, grid/nudge, memory recall and keypad Enter. Confirm keypad typing still works in text fields.
2. Open and hide the separate mixer with Command+= from both windows and while editing text. Check native focus, close/reopen and small-screen scrolling.
3. Select the intended input and output devices. Speak into Audio 1's assigned input before arming. Verify live input meters, then monitoring, recording and playback on the actual interface.
4. Assign a drum library or load samples manually. Audition a long sample at normal and reduced pitch, then trigger a short hit. Check that the long hit continues.
5. Put an effect on one pad. Check that another pad stays dry. Check bypass, removal, undo and both built-in and native editors. Save and reopen with pad effects and verify sound, parameters and native state.
6. Verify browser groups, format filters, precise insert drops, Edit fader/pan, MIDI grid positioning, automation and a short bounce.

Known limits:
Complete Pro Tools keyboard/gesture parity remains unfinished, including the remaining modifier-nudge combinations and keypad Shuttle mode. Drum Rack does not yet reproduce the full Ableton sample editor, choke/return routing or chain reordering. Runtime native plugin processing is still in-process. The earlier test-process SIGSEGV was not independently reproduced or conclusively diagnosed.

Source syntax and manifest checks were performed. Rust compilation, regression execution, visual rendering, Mac launch, native plugins and microphone/interface behavior were not tested here. This is source for diagnostic testing, not a production-readiness certification.
