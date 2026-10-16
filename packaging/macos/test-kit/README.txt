GDAW Mac Test Kit v6.1

Source revision: 964ecaa999ad8feb17bce77c3830005ba1e4b656
Repository: https://github.com/GeraldCerda/GDAW

Verified on Apple Silicon macOS with Rust 1.99.1:
- Native arm64 desktop executable: normal locked release build, deployment target 11.0, zero compiler errors or warnings.
- Complete DSP, model, engine, mix, playback and UI regression suites: 819 passed, 0 failed, 0 ignored, across 87 result targets.
- Regression executables use release optimization with test-only link-time optimization disabled. The application keeps its normal release link settings.
- Packaging lint passed.

Build result:
https://github.com/GeraldCerda/GDAW/actions/runs/38855525806
Regression result:
https://github.com/GeraldCerda/GDAW/actions/runs/38855525758
Full failure audit and remaining untested areas:
https://github.com/GeraldCerda/GDAW/blob/main/docs/GDAW_BUILD_FAILURE_AUDIT_2026-10-16.md

Replace the three kit files in ~/Downloads/GDAW_Test_Kit. Existing source folders, logs, settings and Cargo caches can remain.

Build, install and launch the verified source:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh"

The automated regression suites have already passed in CI. To repeat all six suites locally before installation:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh" --verify

Both verification options disable link-time optimization for test executables only, matching CI and avoiding repeated whole-program optimization for dozens of tests. The application build retains its normal release configuration. A failed compilation or test still stops installation. Formatting differences are recorded separately. --verify-ui runs only UI tests and is insufficient for a complete core regression pass.

The script checks Apple Command Line Tools, installs or updates Rust when needed, downloads the exact pinned source, builds the native application and signs it locally. It installs GDAW Test.app in /Applications, reveals it in Finder, and keeps a previous tester as a timestamped backup. macOS may request your administrator password to move the app into /Applications. The launcher prefers /Applications and falls back to the older ~/Applications location. Test settings remain in ~/Library/Application Support/GDAW Test. Logs remain in this kit's logs directory.

If v6 already installed the app in ~/Applications, open that folder in Finder and move GDAW Test.app into /Applications. Authenticate if macOS asks. This moves the existing build; no rebuild is needed. Preserve any existing app when Finder asks about a name conflict.

The CI build did not launch the application, create or install an app bundle, or exercise physical audio hardware. Installation, signing, interactive use on your Mac, Apollo input/output and recording, device-clock drift, hosted plugins and real-time deadlines still need validation. This is not a production-readiness certification.

Architecture work remains incomplete: complete parameter Touch/Latch gestures and native hosted automation events; timestamp-driven capture alignment; remaining callback/plugin lifecycle and streaming work; delay-capacity replenishment; AudioSuite recovery, partial-selection/crossfaded replacement and media ownership; instrument DSP ordering and overflow presentation. See the linked audit for the exact verification limits.

Use a disposable session copy for playback/stop/seek, recording, short bounce, automation, aux routing, insert/instrument changes and AudioSuite undo/save/reopen. Keep the build or launch log if anything fails.
