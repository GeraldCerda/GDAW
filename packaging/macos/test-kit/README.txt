GDAW Mac Test Kit v5

Source revision: e253e1a612f46cacb9397c3e628403cc6db809a5
Repository: https://github.com/GeraldCerda/GDAW

This is an uncompiled diagnostic test candidate. Architecture hardening remains incomplete. No binary, application, installer, compiler or regression suite was run while preparing this download.

Extract this kit into ~/Downloads/GDAW_Test_Kit, replacing its three kit files. Existing source folders, logs, settings and Cargo caches can remain.

Build, run the regression suites, then install and launch:
bash "$HOME/Downloads/GDAW_Test_Kit/build_test.sh" --verify

The full verification option runs the DSP, model, engine, mix, playback and UI suites. A failed compilation or test stops before replacing the installed tester. Formatting differences are recorded separately. The --verify-ui option checks only UI tests and is insufficient for this architecture pass. Running without a verification option builds and installs without running tests.

The script checks Apple Command Line Tools, installs or updates Rust when needed, downloads the pinned GitHub source, builds the native Mac application and signs it locally. It installs GDAW Test.app in ~/Applications and keeps a previous tester as a timestamped backup. Test settings remain in ~/Library/Application Support/GDAW Test. Build and launch logs remain in this kit's logs directory.

Source changes included:
- Prepared mixer/session handoffs, coalesced control-thread updates, bounded command mailboxes and deferred state reclamation.
- Prepared delay storage, preserved compatible histories and 128-frame transitions between delay taps.
- Per-sample plugin automation fallback, transitive aux solo, routing diagnostics and bus-targeted master processing.
- Monitoring drift correction and underflow accounting.
- Fader Touch return ramps.
- AudioSuite media staging before session publication, with before/ready recovery snapshots.
- Instruments presented through ordinary insert cells and the bottom device chain.

After compilation and regression checks pass, test with a disposable copy of a session. Check playback/stop/seek, interface input and recording, a short bounce, automation, aux routing, insert changes, instrument selection, and AudioSuite undo/save/reopen. Keep the build or launch log if anything fails.

Remaining implementation includes timestamp-based recording alignment, complete parameter gesture semantics, native hosted automation events, automatic delay-capacity replenishment, plugin lifecycle and streaming work, automatic AudioSuite recovery and partial-selection replacements, and instrument DSP ordering/overflow presentation. Hosted plugin behavior, PDC transitions, UI interaction and all hardware/platform behavior remain unvalidated.

See docs/GDAW_CORE_CONTINUATION_2026-10-15.md in the downloaded source for the detailed changes, limitations and unperformed checks.
