# GDAW Edit/Mix audit — 2026-10-14

Scope: the 3:37 PM Mac screenshot, main at b547d7ab02abfbe2324cb90b6ce281fb0ac85977, Edit/Mix rendering, plugin browser and insert menus, device chain, startup defaults and UI regression fixtures. This is a source and screenshot audit, not a compiled-app or hardware certification.

## Findings addressed in this pass

| Finding | Source change | Verification available here |
|---|---|---|
| Only an instrument selector appears beside MIDI 1; Audio 1 has no equivalent | New sessions expose Inserts A–E and Sends A–E alongside Instrument and I/O. Both tracks get effect slots. Instruments stay specific to instrument tracks | Startup and actual UI accessibility regressions added |
| Medium tracks silently omit slot E | Five bounded cells at every supported height; full-width labels at medium height, distinct compact cells on short tracks | Geometry regression covers seven heights and both column widths |
| Empty slots are hard to distinguish | A–J insert letters, bordered empty cells, full track/slot/plugin tooltips; occupied names and bypass faces retained | Real-frame slot-count regression added; visual runtime review pending |
| Browser drops cannot address a specific insert | Edit and Mix slots accept effect/preset drops at that exact track/slot, through existing undoable commands | Both starter track types, non-adjacent slots, undo and rejected drops covered |
| Opening an insert leaves the bottom panel on another track | Clicking an insert selects its track and exposes its devices | Source inspection; Mac interaction check pending |
| Header drops lack a plugin destination | Track header backgrounds accept browser items and select the destination | Source inspection; pointer check pending |
| Insert menus organize plugins differently from the browser | Shared developer/effect groups and format filters; search respects those filters and metadata. AU/VST3 remain separate labelled choices | Existing search/menu regressions updated; catalog regressions retained |
| Native discovery can block insert submenus | Menus consume the shared asynchronous catalog; removed synchronous per-format submenu discovery. Existing per-bundle helper isolation retained | Source inspection; native scan verification pending |
| Edit I/O volume and pan are display-only | Volume popup with fader/numeric entry; mono/stereo pan controls; volume Option-click reset; existing engine commands and merge keys | Popup regression added; audio/automation testing pending |
| Mixer sections can overlap faders at short heights | Reserve height for enabled sections and scroll in both directions; instrument section visible by default | Height regression added; Mac resize testing pending |
| Eighth-note grid value starts in Slip mode | New GDAW sessions start in absolute Grid mode with existing narrow eighth-note lines | Startup regression extended; saved session modes preserved |
| Old tester preferences keep columns hidden | One-time migration exposes inserts/sends and mixer instruments, preserving later choices | Migration regression added |
| Regression fixtures expect the old dark default and embedded Mix command | Light expectation corrected; isolated rendering fixtures explicitly select Mix; command tests still assert separate mixer behavior | Source inspection; tests updated but unexecuted |

The screenshot confirms the light palette, left browser, bottom Drum Rack, named devices, track headers and eighth-note selector are present. It does not prove mic levels, recording, plugin DSP, shortcuts or routing work.

## Remaining gaps

- Full Mac shortcut parity is unfinished. Numeric keypad identity and keypad Enter need a native input bridge. Existing row-key tools, focus shortcuts and Cmd+= do not constitute the complete Pro Tools shortcut set.
- The separate mixer toggle exists; foreground/focus and close/reopen behavior need macOS native-window testing.
- Instrument tracks have a dedicated instrument plus ten effect slots. Audio tracks have ten effect slots. A MIDI generator does not belong in the audio recording track merely to make two columns look identical.
- Drum Rack has sixteen pads, assignments, audition, gain/pan/tune/mute, waveform and optional random library loading. Full Ableton chain editing, per-pad effects and its complete sample editor remain absent.
- Browser and device areas are custom hybrid elements by request. Exact Pro Tools spacing, toolbar variants, dialogs and every menu/gesture are not achieved.
- Discovery isolation is not a realtime DSP sandbox. An instantiated plugin can still destabilize the app.
- Mic routing, meters, recording, monitoring latency, channel pairing, hardware outputs, automation playback and native editors require a Mac/interface test. The empty meter on a MIDI track in this screenshot does not establish a mic-input defect on Audio 1.
- Broader upstream gaps including session interchange, external synchronization and control surfaces remain. No claim of complete workstation parity or production readiness is made.

## Verification boundary

No clone, Cargo build, Rust test run, app launch, installer or binary was produced here, as requested. Selected source files were checked with a Rust syntax parser; this does not type-check Rust or validate egui behavior. Added/updated regressions must run on the tester's Mac. A follow-up tester-script commit pins this source revision and reuses the existing Cargo cache on the user's Mac.

Reference: [Avid Edit Window overview](https://www.avid.com/pro-tools/user-guide/edit-window), including configurable inserts and I/O. Original code retained; no proprietary assets copied.

## Follow-up: Drum Rack reliability

Base reviewed: 0ccdd1ad7e4820c1cb8cd8f8422226041136622f.

| Additional finding | Source correction | Regression coverage added |
|---|---|---|
| Library directory traversal runs in UI logic | Native discovery runs in a named worker with bounded traversal, cancellation checks and non-blocking result polling | Worker discovery, empty-file/symlink filtering |
| A previous folder's scan can outlive a library change | Cancelling or changing the assignment drops its receiver and cancels its worker; stale results never enter the new catalog | Clearing and switching libraries preserve the session and reject old results |
| Scan errors resemble an empty library | Root errors and worker failures have explicit status messages; the device area offers Rescan drum library | Empty-library integration fixture waits for completion |
| One corrupt file rejects every random pad | Automatic/random kits opt into skip_invalid, report skipped files and fill consecutive available pads; explicit loads remain strict by default | Partial success, preserved assignments, one-step undo and all-invalid replacement rollback |
| Previewing Rack A, Rack B, then Rack A can revive Rack A's suspended one-shot | Switching stopped preview tracks resets the previous instrument before triggering the next; playback instruments remain sounding during transport playback | Actual mix rendering tests for stopped and playing track switches |
| Playback/seek retains an obsolete preview countdown | Play and Locate clear the preview timer; preview commands set stopped/playing mixer mode before triggering | Playback command-queue regression |
| Replacing a sample leaves old MIDI events queued | Sampler clears pending events when its sample changes or is removed, in addition to stopping existing voices | DSP rendering regression for replacement and clearing |

No additional audio-callback allocation or lock was introduced by these preview changes. Folder scanning is asynchronous in the native application; sample decoding/attachment still uses the existing engine command path on the UI thread. Web discovery retains its existing synchronous filesystem limitation.

Rust syntax parsing is the available check here. The added Rust regressions have not been compiled or executed. Mic/interface checks, native window focus, full keypad shortcuts and per-pad effect chains remain outstanding. Stopped pad previews still have the existing two-second duration limit; complete long-sample audition remains unfinished.
