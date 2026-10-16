GDAW Mac Test Kit v8 — source candidate

Repository: https://github.com/GeraldCerda/GDAW
Exact package revision: SOURCE_COMMIT beside this file.

The update ZIP includes the full source at source/GDAW-<SOURCE_COMMIT>, plus the matching build and launch scripts. The source files match that Git revision. It does not reuse an older source pin. The script downloads source only if that exact source directory is missing. Rust and Cargo dependencies may require downloads.

Build, install and launch:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh"

The normal command runs no regression suites. It builds the native Mac application, signs it locally, installs GDAW Test.app into /Applications, reveals it in Finder and launches it. macOS may request your administrator password. A previous tester in /Applications is preserved as a timestamped backup. Existing kit logs, preferences and Cargo caches remain available.

The installed app records the exact source revision as GDAWBuildCommit in Contents/Info.plist. Preferences use ~/Library/Application Support/GDAW Test. Build and launch logs use this kit's logs directory.

Source changes in this candidate:
- Startup plugin discovery, concurrent AU/VST3/CLAP enumeration, a startup progress screen, persisted catalog and isolated-scan descriptor/failure caches, explicit scan failures and rescan support.
- User-first VST3/CLAP search precedence; network and application-bundled VST3 locations on macOS.
- Automatic editor opening after insertion, AUv2 native Cocoa views and parameter gestures, bounded VST3/CLAP edit and gesture queues, and CLAP parameter flushing while stopped.
- Instrument processing follows its displayed insert position. Legacy overflow K is visible in Edit and Mix as well as the bottom device chain.
- Named hardware output mixing and sends, physical Aux/Folder input monitoring, plugin key-input and additional-output routing through internal buses, graph/solo handling, missing-route diagnostics and bus-delete cleanup.
- Pan/mute/send/plugin Touch and Latch writing, Touch return ramps, native editor gesture forwarding and mute-lane override fixes.
- Capture/output timestamp anchors for take placement and trimming of captured material before timeline zero.
- AudioSuite partial-selection fragments, preservation of their editable clip effects, and durable copies of previously memory-only original media in transaction snapshots.

This revision has NOT been compiled, type/borrow checked, linted, linked, installed or launched. No new or existing Rust tests have run. The added test source is unexecuted. Static source review, TOML parsing, shell syntax checking and Git/package byte comparisons do not establish a clean application build.

Still incomplete:
- End-to-end callback allocation/lock/deadline instrumentation, remaining hosted-plugin lifecycle paths, shutdown ownership and bounded disk streaming.
- Prepared delay-capacity replenishment and continuity beyond existing capacity; native sample-offset automation delivery instead of the existing one-sample fallback; complete gesture timing and parameter overrides between UI ticks.
- Continuous timestamp-based device drift correction, loop/seek/varispeed recording alignment, converter-latency calibration, and phase-accurate compensation of additional plugin outputs and all sidechain branches.
- Master-track sidechains, plugin port layouts beyond the exposed main/first auxiliary input and supported extra outputs, and dynamic port-layout renegotiation.
- AudioSuite preview, automatic recovery discovery/replay, replacement crossfades, media ownership/garbage collection, and a permanent project location for unsaved-session transaction media. Partial selections that change length require a full clip selection. Missing original media without a pool copy cannot be recovered.
- AAX, VST2 and asynchronously instantiated AUv3 plugins are unsupported. External MIDI-device recording/routing was not implemented or validated by this update.

Untested behavior includes every new and existing Rust test; real AU/VST3/CLAP discovery, editors, automation and plugin states; Apollo and other physical I/O, recording offsets and drift; callback deadlines and saturation; audible delay transitions, routing null/phase tests and offline parity; AudioSuite failure/crash/reopen/undo behavior; UI rendering/interaction; Mac signing, /Applications installation and launch; Windows/Linux/web builds.

Optional verification is explicit:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh" --verify

--verify runs DSP/model/engine/mix/playback/UI suites. --verify-ui runs UI suites only. Both disable LTO for test executables only. The application retains its normal release link settings. Neither option runs by default.
