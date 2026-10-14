GDAW Mac Test Kit v2

Source revision: 7c4a32f074f92694e05159e94dc25e8c28aa52d4
Repository: https://github.com/GeraldCerda/GDAW

This kit contains source-download, build and launch scripts. It downloads the complete pinned source from your fork when you run it on your Mac. No binary is included and no DAW build was performed while preparing this kit.

Save the ZIP to Downloads and run the supplied Terminal command. The script installs Rust if needed, checks Apple Command Line Tools, downloads the pinned source, compiles the native Mac app, generates a Mac icon from Purple G icon with transparent background.png, creates and locally signs GDAW Test.app, and launches it.

The app goes to ~/Applications/GDAW Test.app. Previous tester copies are renamed as timestamped backups. Settings, presets and autosaves used by launch_test.sh go to ~/Library/Application Support/GDAW Test. Always launch with launch_test.sh to keep this isolated profile. Double-clicking the app directly uses the default profile.

New sessions start with MIDI 1 and its 16-pad Drum Rack, plus mono Audio 1 assigned physical input 1. Input and output columns, a left browser, and the bottom selected-track device chain are visible. Cmd+= opens or focuses a separate mixer while preserving the Edit window. The main window and app menu are called G; the test bundle remains GDAW Test for easy identification.

The Drum Rack starts with empty pads. Load your own samples through Load, the sample browser, the clip list, or a Finder drop onto a pad. Assigned pads support stopped-transport audition, gain, pan, tuning and mute. Built-in devices have inline controls in the bottom chain. Hosted plugins open native editor windows. The browser supports device search, sample folders, saved presets and loading devices onto the selected track. Warp, formant and clip-pitch automation work is outside this change.

Commands after extraction:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v2/build_test.sh"
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v2/launch_test.sh"

Optional regression run on your Mac:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v2/build_test.sh" --verify

Optional mode reports formatting results, then compiles and runs DSP, model, engine, mix, playback and UI regression suites before building the app. A failing test stops the script. Default mode compiles the desktop app without running these tests.

If Apple Command Line Tools are missing, finish the installation dialog and rerun the command. Network access is required for GitHub, craft-libs, crates.io and Rust downloads. Dependencies use Cargo.lock. Logs are retained inside the kit's logs folder; share the complete build log after a failure. The source archive and build cache remain available for retries.

Manual checks:
1. Confirm MIDI 1, Drum Rack, Audio 1 input 1, visible I/O columns, browser, bottom devices and Purple G icon on startup.
2. Press Cmd+= from Edit and while a text field is focused. Confirm a separate mixer opens or focuses and Edit stays visible. Close the mixer and reopen it.
3. In Setup > Playback Engine select the intended input/output devices, rate and buffer. Confirm the available physical input channels. Record-arm Audio 1 and verify live input metering. Enable Monitor to hear input; disable it when using interface direct monitoring.
4. Load different samples on two Drum Rack pads. Click each pad to audition with the playhead stopped. Confirm the correct sample and pad controls; check parameter undo.
5. Create a MIDI clip on MIDI 1 and program notes 36..51. Check quarter, sixteenth and triplet snapping, dragging, resizing, quantization and selection across clip changes.
6. Search for an effect in the browser, drag it into Track Devices, and double-click another effect. Confirm devices append without replacing earlier inserts. Adjust their controls and undo.
7. Save and reopen a session with pad assignments and effects. Verify samples, MIDI playback, routing and device settings survive.
8. Record two audio tracks on distinct physical inputs. Confirm isolated takes and correct channel assignments after reopening. Unavailable/no-input recording must report a routing error. Internal bus recording remains unavailable.
9. Open a hosted plugin editor, adjust track volume/pan and clip gain, and bounce a short selection. Inspect meters and audio output.
10. Check the layout on your actual screen size and verify no panels obscure the timeline or essential controls.

Ready for diagnostic testing only. Rust compilation and tests, macOS launch, microphone permissions, hardware I/O, plugin hosting and icon conversion were not executed during preparation. Added regression tests are present in source but unrun here.
