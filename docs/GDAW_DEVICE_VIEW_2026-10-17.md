# Arrangement and Device View source pass — 2026-10-17

Base: d989ea1e80ca8d89470fd261c3eb2f3629934951, after the uploaded splash artwork. This source pass follows the user's Ableton Arrangement screenshot, while retaining the Pro Tools-style Edit insert columns and mixer. No proprietary artwork or app resources were copied. The existing neutral-grey appearance stays in use.

| Area | Previous behavior | Source change |
| --- | --- | --- |
| Bottom area | Cards and a global routing toolbar; Browser occupied the left part of the bottom region | Full-width bottom panel with a resizable left information pane, compact device headers, large X/Y pads, parameter selectors, optional configured-control panels and a wide blank effects drop target |
| Device parameters | First nine parameters presented as rotary knobs | Real normalized X/Y parameter edits, per-instance axis mappings and up to 64 chosen controls; Configure can collect native-editor edits |
| Device persistence | Plugin state and host parameters persisted | New controls metadata is serialized with each Insert and follows its existing clone/move/copy paths; older sessions default to no mappings |
| Header actions | Activator, title and separate open-window row | Activator, parameter expansion, editor access, draggable/foldable title, Browser replacement and existing parameter-preset menu |
| Replacement | Regular insert assignment could leave an old slot's automation/routes | New guarded replacement command clears those assignments, returns affected native instance addresses and participates in engine undo |
| Loop | Followed edit selection; playback and record loop flags could differ | Saved independent loop brace, shared playback/record range, Cmd/Ctrl+L from selection, draggable edges/body and editable start/length in seconds |
| Ordinary Play | Could stop at selection end | Continues from cursor; explicit Play Selection stays bounded |
| Record | Implicit selection punch; Record off stopped playback | Arrangement Record continues from cursor; Record off finalizes capture and continues playing; Shift-Record arms until Play; F9 records |
| Loop takes | Equal-size passes from record start; very short final passes dropped | Partial first pass placed up to loop end, subsequent passes at loop start, final fragment retained, take lanes shown and cycle geometry autosaved |
| Monitoring | One input-monitor bool | In/Auto/Off for audio tracks; new tracks use Auto and older serialized tracks retain the bool behavior |
| Arming | Clicking a track could accumulate armed tracks | UI arm clicks request exclusive arm; Cmd/Ctrl-click adds tracks; grouped recording behavior remains; API exclusivity is optional |

Changes to loop geometry, seeking and playback speed are refused during recording because the captured file has one frozen placement plan. At 1,000 take lanes, the remaining captured source is preserved as one continuous final clip with a message rather than silently discarded. The live-take display reads the same frozen cycle plan. Recording offsets still depend on the existing input/output clock mapping.

Regression source was added for independent loops, Record arming, playback continuation, mapping validation/moves/serialization, guarded replacement, monitoring, exclusive arm, partial passes and bounded final takes. This source has not been executed.

References consulted:

- https://www.ableton.com/en/live-manual/12/using-plug-ins/
- https://www.ableton.com/en/live-manual/12/arrangement-view/
- https://www.ableton.com/en/live-manual/12/recording-new-clips/
- https://www.ableton.com/en/live-manual/12/routing-and-i-o/

## Verification limits and remaining work

No compilation, Rust type/borrow checking, linting, linking, Rust test execution, UI rendering, app launch, audio-device operation, installation or installer generation was performed. Static review and remote byte comparisons cannot establish a clean build, working interaction geometry or audible correctness.

Untested changes include native-editor synchronization and gestures, XY taper and endpoint behavior, Configure capture, per-device controls serialization, device/preset/replacement actions and rollback/undo, state recreation, full-width and separate-Mix layout, clipping/scrolling/resizing, loop dragging/snapping/toggling, keyboard dispatch, Record-off continuity, partial/short loop takes, take-lane selection and recovery, and Auto monitoring/exclusive arming. Existing hardware drift/latency, callback deadlines, routing compensation, AudioSuite recovery and hosted-plugin verification limits still apply; see the earlier core continuation handoffs and the test-kit README.

This is not complete Ableton parity. Session clip launching/recording, external MIDI recording, MIDI overdub, count-in scheduling, explicit Arrangement punch-in/out controls, editable bar/beat/tick loop fields, comping, tempo-warped sample loops, unlimited/parallel racks and MIDI effects remain separate unfinished work. Parameter presets still store published host parameters, not complete opaque third-party plugin state. The processing model still has ten insert positions and one instrument per instrument track. Automatic input/clip monitoring decisions occur at render-block boundaries. The screenshot layout has not been visually verified in the application.

The matching source-only Mac kit compiles on the user's Mac, then signs, installs and launches /Applications/GDAW Test.app. Its normal command does not run regression suites. The package's external SOURCE_COMMIT names the exact published source revision; no older tester pin is substituted.
