GDAW Mac Test Kit v3

Source revision: f68e05bd302f72c63808997936e59380d251a8f7
Repository: https://github.com/GeraldCerda/GDAW

This kit contains source-download, build and launch scripts. It downloads the complete pinned source from your fork when you run it on your Mac. No binary is included and no DAW build was performed while preparing this kit.

Save the ZIP to Downloads and run the supplied Terminal command. The script installs Rust if needed, checks Apple Command Line Tools, downloads the pinned source, compiles the native Mac app, generates a Mac icon from Purple G icon with transparent background.png, creates and locally signs GDAW Test.app, and launches it.

The app goes to ~/Applications/GDAW Test.app. Previous tester copies are renamed as timestamped backups. Settings, presets and autosaves used by launch_test.sh go to ~/Library/Application Support/GDAW Test. Always launch with launch_test.sh to keep this isolated profile. Double-clicking the app directly uses the default profile.

New sessions start with MIDI 1 and its 16-pad Drum Rack, plus mono Audio 1 assigned physical input 1. Input and output columns, a left browser, and the bottom selected-track device chain are visible. Cmd+= opens or focuses a separate mixer while preserving the Edit window. The main window and app menu are called G; the test bundle remains GDAW Test for easy identification.

Plugin browsing defaults to developer groups. Switch Group by to Effect for compressors, EQs, limiters, gates, reverbs and the other reported categories. Filter by AU, VST3, CLAP or Built-in without collapsing distinct plugin versions. Search covers names, developer, effect group and format. Installed plugins are scanned in a background thread; Rescan refreshes the list. Plugins without usable classification remain in Other Effects or Dynamics. Matching AU/VST3 names and developers share the most specific available category.

Physical audio input capture now starts for assigned audio tracks even before record enable. Dedicated pre-processing input meters remain visible during playback; stopped track meters also show the input. Enabling Monitor is still explicit. Input capture requests the widest supported channel configuration at the chosen or default input rate. Routing changes clear stale input peaks. Device names, opened channel counts and capture failures are visible in Audio Setup, I/O Setup and Track Devices.

The Drum Rack starts with empty pads. Load your own samples through Load, the sample browser, the clip list, or a Finder drop onto a pad. Assigned pads support stopped-transport audition, gain, pan, tuning and mute. Built-in devices have inline controls in the bottom chain. Hosted plugins open native editor windows. The browser supports device search, sample folders, saved presets and loading devices onto the selected track. Warp, formant and clip-pitch automation work is outside this change.

Commands after extraction:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v3/build_test.sh"
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v3/launch_test.sh"

Optional regression run on your Mac:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v3/build_test.sh" --verify

Optional mode reports formatting results, then compiles and runs DSP, model, engine, mix, playback and UI regression suites before building the app. A failing test stops the script. Default mode compiles the desktop app without running these tests.

If Apple Command Line Tools are missing, finish the installation dialog and rerun the command. Network access is required for GitHub, craft-libs, crates.io and Rust downloads. Dependencies use Cargo.lock. Logs are retained inside the kit's logs folder; share the complete build log after a failure. The source archive and build cache remain available for retries.

Manual checks:
1. Confirm MIDI 1, Drum Rack, Audio 1 input 1, visible I/O columns, browser, bottom devices and Purple G icon on startup.
2. Press Cmd+= from Edit and while a text field is focused. Confirm a separate mixer opens or focuses and Edit stays visible. Close the mixer and reopen it.
3. In Setup > Playback Engine select the intended input/output devices, rate and buffer. Confirm the available physical input channels. Before arming Audio 1, speak into the assigned physical input and verify live input metering. Check the actual input device name and channel count. Change the input assignment and confirm the meter follows that channel. Enable Monitor to hear input; disable it when using interface direct monitoring.
4. Load different samples on two Drum Rack pads. Click each pad to audition with the playhead stopped. Confirm the correct sample and pad controls; check parameter undo.
5. Create a MIDI clip on MIDI 1 and program notes 36..51. Check quarter, sixteenth and triplet snapping, dragging, resizing, quantization and selection across clip changes.
6. Expand a developer group, switch to Effect, and filter AU then VST3. Check that compressors and EQs are grouped with similar devices and both formats remain separately selectable. Search by developer and effect, then drag an effect into Track Devices and double-click another. Confirm devices append without replacing earlier inserts. Adjust their controls and undo.
7. Save and reopen a session with pad assignments and effects. Verify samples, MIDI playback, routing and device settings survive.
8. Record two audio tracks on distinct physical inputs. Confirm isolated takes and correct channel assignments after reopening. Unavailable/no-input recording must report a routing error. Internal bus recording remains unavailable.
9. Open a hosted plugin editor, adjust track volume/pan and clip gain, and bounce a short selection. Inspect meters and audio output.
10. Check the layout on your actual screen size and verify no panels obscure the timeline or essential controls.

Ready for diagnostic testing only. Rust compilation and tests, macOS launch, microphone permissions, hardware I/O, plugin hosting and icon conversion were not executed during preparation. Added regression tests are present in source but unrun here.
