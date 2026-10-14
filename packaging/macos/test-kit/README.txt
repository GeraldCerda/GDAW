GDAW Mac Test Kit v4

Source revision: 75f9e4d51fe5f5b47bb9a26f79d11eee056fd7f5
Repository: https://github.com/GeraldCerda/GDAW

This kit contains source-download, build and launch scripts. It downloads the complete pinned source from your fork when you run it on your Mac. No binary is included and no DAW build was performed while preparing this kit.

Save the ZIP to Downloads and run the supplied Terminal command. The script installs Rust if needed, checks Apple Command Line Tools, downloads the pinned source, compiles the native Mac app, generates a Mac icon from Purple G icon with transparent background.png, creates and locally signs GDAW Test.app, and launches it.

The app goes to ~/Applications/GDAW Test.app. Previous tester copies are renamed as timestamped backups. Settings, presets and autosaves used by launch_test.sh go to ~/Library/Application Support/GDAW Test. Always launch with launch_test.sh to keep this isolated profile. Double-clicking the app directly uses the default profile.

New sessions start with MIDI 1 and its 16-pad Drum Rack, plus mono Audio 1 assigned physical input 1. Input and output columns, a left browser, and the bottom selected-track device chain are visible. Cmd+= opens or focuses a separate mixer while preserving the Edit window. The main window and app menu are called G; the test bundle remains GDAW Test for easy identification.

Plugin browsing defaults to developer groups. Switch Group by to Effect for compressors, EQs, limiters, gates, reverbs and the other reported categories. Filter by AU, VST3, CLAP or Built-in without collapsing distinct plugin versions. Search covers names, developer, effect group and format. Installed plugins are scanned in a background thread; Rescan refreshes the list. Plugins without usable classification remain in Other Effects or Dynamics. Matching AU/VST3 names and developers share the most specific available category.

Physical audio input capture now starts for assigned audio tracks even before record enable. Dedicated pre-processing input meters remain visible during playback; stopped track meters also show the input. Enabling Monitor is still explicit. Input capture requests the widest supported channel configuration at the chosen or default input rate. Routing changes clear stale input peaks. Device names, opened channel counts and capture failures are visible in Audio Setup, I/O Setup and Track Devices.

The Drum Rack starts with empty pads. Load your own samples through Load, the sample browser, the clip list, or a Finder drop onto a pad. Assigned pads support stopped-transport audition, gain, pan, tuning and mute. Built-in devices have inline controls in the bottom chain. Hosted plugins open native editor windows. The browser supports device search, sample folders, saved presets and loading devices onto the selected track. Warp, formant and clip-pitch automation work is outside this change.

Commands after extraction:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v4/build_test.sh"
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v4/launch_test.sh"

Default build mode installs and launches after compilation. Optional verification deliberately stops before installation if any test fails. To reuse a v3 build cache, replace its build_test.sh with the current pinned script and run it without --verify.

Fade controls now use the visible editor bounds for drawing and hit-testing. Headless editor tests do not automatically scan native plugins. The reported test-process SIGSEGV has not been independently reproduced or conclusively diagnosed.

Optional regression run on your Mac:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit_v4/build_test.sh" --verify

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

Current UI update:
The rack uses Ableton-style pad order and octave labels, with C1 at MIDI note 36 and D#2 at note 51. The empty rack has a clickable library-assignment area to its right. Choose your own drum sample folder; the folder persists in preferences. Empty racks receive up to 16 distinct randomly chosen samples from that folder. Existing pad assignments are preserved. Load random kit explicitly replaces the kit in one undo step. Library traversal and file sizes are bounded; no audio files are bundled or fetched externally. A populated rack shows the selected pad waveform, sample loading, tuning, pan, gain and mute controls.

Light appearance is the default, with a one-time migration of old appearance preferences. New GDAW sessions use an eighth-note grid and Bars|Beats. Subdivision lines are half-point strokes and remain visible down to two pixels between lines. Existing session grid settings remain saved with each session.

Cmd+= now shows or hides the separate mixer while keeping Edit in the main window. Mac corrections include Cmd+1..7 tools, Smart Tool F-key combinations, Cmd+[ and Cmd+] zoom, Commands Focus R/T zoom, Control+1..5 zoom presets, Control+E Zoom Toggle, Return navigation, Control+Shift+S search, Control+Shift+K shortcut list and MIDI/Score editor shortcuts.

Complete Pro Tools shortcut parity is unfinished. The egui input layer does not preserve all number-row versus keypad distinctions. Keypad-only windows and keypad Enter require a native input bridge; their labels identify the intended keypad assignments. Commands for features absent in GDAW remain unavailable. This update does not claim a complete reproduction of Ableton Drum Rack, per-pad effect chains or its entire sample-editor feature set.

Native VST3/CLAP discovery remains isolated in per-bundle helper processes. Runtime plugin processing after insertion remains in-process.

Regression tests were added but not executed during source preparation. This source revision needs a Mac compile, UI checks and microphone/hardware testing.
