GDAW Mac Test Kit

Reviewed source revision: b5ef359a5a1860541fc352deea40bbd6a03f1d3d
Repository: https://github.com/GeraldCerda/GDAW

This kit contains build and launch scripts. It downloads the complete source archive for the exact revision above from your fork when you run it on your Mac. No DAW binary is included. No DAW build was performed while preparing the kit.

Save the ZIP to Downloads. Run the Terminal command supplied with the ZIP. The script installs Rust if needed, checks Apple Command Line Tools, downloads the pinned source, compiles the native Mac app, creates and locally signs GDAW Test.app, and launches it.

The app goes to ~/Applications/GDAW Test.app. Previous copies of this tester are renamed as timestamped backups. Settings, presets and autosaves used by launch_test.sh go to ~/Library/Application Support/GDAW Test. Always launch with launch_test.sh to keep this isolated profile. Double-clicking the app directly uses the upstream default profile.

The app's windows and menus still carry some SoundCraft branding. This kit does not implement the missing Drum Rack, persistent plugin/sample browser, or warp/formant/clip-pitch automation workflow.

Commands after extraction:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit/build_test.sh"
bash "$HOME/Downloads/GDAW_Mac_Test_Kit/launch_test.sh"

Optional regression run on your Mac:
bash "$HOME/Downloads/GDAW_Mac_Test_Kit/build_test.sh" --verify

The optional mode reports formatting results, then compiles and runs the model, engine, mix, playback and UI regression suites before building the app. A failing test stops the script. Default mode only compiles the desktop app and does not run these tests.

If Apple Command Line Tools are missing, the installation dialog opens. Finish installation and rerun the same command.

Network access is required for GitHub, the pinned craft-libs dependency, crates.io and Rust downloads. The package uses Cargo.lock without updating dependencies.

Logs are retained inside the kit's logs folder. After a failure, share the complete build log. The source archive and build cache remain inside the kit so retries can reuse them.

Manual checks:
1. Open Setup > Playback Engine. Select the intended input and output device, rate and buffer. Confirm the opened input channel count.
2. Create mono and stereo audio tracks. In Setup > I/O assign different physical inputs. Verify record-arm alone shows live input levels without enabling software monitoring.
3. Enable Monitor explicitly and verify the assigned input is heard. Disable it when using interface direct monitoring. Verify there is no duplicated live signal from record-arm alone.
4. Choose no input or an unavailable input and attempt recording. Expect an actionable routing error rather than recording microphone channel 1. Internal bus recording is not implemented.
5. Record two tracks simultaneously. Stop, audition each take separately, save the session and reopen it. Confirm correct channel assignments, take lengths and file names.
6. Create a MIDI clip. Select quarter, sixteenth and triplet grids. Double-click empty space to add a note. Drag notes, resize their ends, and quantize. Check that resizing matches its preview.
7. Select notes, move them past other notes, then move them again. Confirm the same notes remain selected.
8. Switch between MIDI clips with notes selected. Confirm selection and unfinished gestures clear. Delete must not remove a note in the newly opened clip until it is selected.
9. Test a 3/4 or 7/8 session and a meter change. Inspect bar lines. Open a long MIDI clip at a fine grid and check responsiveness.
10. Insert an effect, adjust track volume/pan and clip gain, delete a bus, and bounce a short selection. Confirm saved state after reopening.

Ready for diagnostic testing only. Compilation, Rust tests, macOS launch, microphone permissions, device I/O and third-party plugin hosting were not executed during kit preparation.
