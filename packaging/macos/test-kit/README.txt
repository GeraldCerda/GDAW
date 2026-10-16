GDAW Mac Test Kit v7

Repository: https://github.com/GeraldCerda/GDAW
Package revision: read SOURCE_COMMIT beside this file. The source ZIP, kit scripts and app build all use that exact Git commit.

This kit has no embedded older source pin. Each delivery includes a new source ZIP and matching kit ZIP. SOURCE_COMMIT is package metadata created after the Git commit; it does not modify repository source files. An explicit --revision=<full Git commit> selects the same revision when using GitHub downloads directly.

Extract the kit into ~/Downloads/GDAW_Test_Kit, replacing the kit files and SOURCE_COMMIT. Existing logs, settings and Cargo caches remain available.

Build, install and launch:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh"

The normal command runs no regression suites. It downloads the exact revision's source, builds the native Mac application, signs it locally, installs GDAW Test.app into /Applications, reveals it in Finder and launches it. macOS may request your administrator password for installation. Any previous tester in /Applications is preserved as a timestamped backup. The launch script prefers /Applications and retains a fallback to ~/Applications.

The installed app records its exact source commit in Contents/Info.plist as GDAWBuildCommit. Test settings remain in ~/Library/Application Support/GDAW Test. Build and launch logs remain in this kit's logs directory.

Optional verification is explicit:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh" --verify

--verify runs the DSP, model, engine, mix, playback and UI suites. --verify-ui runs only UI tests. Both options disable link-time optimization for test executables only. The application keeps its normal release link settings. No test suite runs without a verification option.

The last native arm64 desktop build and complete core/UI verification were performed at source revision 964ecaa999ad8feb17bce77c3830005ba1e4b656 with Rust 1.99.1: zero compiler errors or warnings; 819 tests passed, zero failed, zero ignored.
Build: https://github.com/GeraldCerda/GDAW/actions/runs/38855525806
Regressions: https://github.com/GeraldCerda/GDAW/actions/runs/38855525758

Subsequent kit changes correct the /Applications destination and bind each package to its own Git revision. These kit changes have not been compiled, installed or launched on a Mac. The older successful CI runs do not establish an installation result for this package.

Architecture work remains incomplete: complete parameter Touch/Latch gestures and native hosted automation events; timestamp-driven capture alignment; remaining callback/plugin lifecycle and streaming work; delay-capacity replenishment; AudioSuite recovery, partial-selection/crossfaded replacement and media ownership; instrument DSP ordering and overflow presentation.

Physical audio, Apollo input/output and recording, device-clock drift, hosted plugins, callback timing, audible delay changes, AudioSuite crash recovery and interactive application use remain unvalidated. Use a disposable session copy.
Full audit: https://github.com/GeraldCerda/GDAW/blob/main/docs/GDAW_BUILD_FAILURE_AUDIT_2026-10-16.md
