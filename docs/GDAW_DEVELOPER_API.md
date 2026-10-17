# Build with GDAW

GDAW exposes the same command engine used by its interface to external developers. The desktop API is optional, local to the computer and disabled unless the app is launched with `--control`. The repository also includes a headless CLI and an MCP bridge. The Python client in [examples/gdaw_client.py](../examples/gdaw_client.py) uses the standard library and this repository's existing licenses.

## Connect to the desktop

Quit the running app first. Launch the executable with a chosen loopback port:

```bash
"/Applications/GDAW Test.app/Contents/MacOS/GDAW-Test" --control 7801
```

To use the test profile, set `SOUNDCRAFT_CONFIG_DIR` to `"$HOME/Library/Application Support/GDAW Test"` before launching. `--control 0` chooses an available port and prints `SOUNDCRAFT_CONTROL_PORT=...`. It does not expose an internet-facing service or start a cloud account. Other programs on the same computer can control an enabled instance; enable it when you want those programs to have that access.

Requests and replies are UTF-8 JSON, one object per line, over TCP at `127.0.0.1`. Every request has an `id`, a `method` and `params`.

```json
{"id":1,"method":"engine.execute","params":{"command":"track.new","params":{"kind":"audio","format":"Stereo","name":"Vocal"}}}
```

```json
{"id":1,"ok":true,"result":{"tracks":[12]}}
```

A rejected operation returns `ok: false` and `error`. Programmatic commands use their parameters and never open a file picker. File reads, writes, plugin loading and playback still have their normal effects. Inspect command availability before execution.

## Discover capabilities and commands

`engine.capabilities` reports API version 1, the native session schema version, formats and known unsupported interchange types. `engine.commands` returns command IDs, parameter documentation, enabled state and the reason for a disabled command. `session.inspect` reports tracks, clips, routing and automation. `session.document`, through `engine.execute`, returns the versioned native JSON document; decoded audio is referenced separately, not embedded.

```python
from gdaw_client import GDAWClient

with GDAWClient(7801) as gdaw:
    print(gdaw.execute("engine.capabilities"))
    print(gdaw.commands("mix."))
    print(gdaw.session(full=True))
```

The client's `batch` sends commands in order and stops on the first error. It is not a transaction and does not roll back earlier commands or filesystem effects. Individual document commands retain the engine's undo behavior. The `mix.insert_move` command validates and publishes a device move as one document operation, including its parameter automation and plugin bus assignments.

## Samples and plugin availability

These methods run on the desktop UI side:

| Method | Parameters | Result |
|---|---|---|
| `library.search` | `query`, optional `limit` up to 200 | Indexed sample paths, favorite status and whether indexing continues |
| `library.preview` | `path` | Queues background decoding and audition through the current output |
| `library.stop_preview` | `{}` | Stops audition without changing song transport |
| `plugins.status` | `{}` | Available catalog, excluded plugin IDs/reasons and scan state |
| `plugins.hide` | `id` | Persists an exclusion from normal plugin pickers |
| `plugins.retry` | `id` | Removes the exclusion and retries a failed instance if it is in the session |
| `export.status` | `{}` | Whether an export runs, its progress when available and the status text |

Configure sample locations in the browser once. Search uses the cached index; refresh runs in the background. Preview decodes up to 30 seconds, with a decoded-sample bound and a 128 MiB source-file limit. Full imports use the existing import path. Preview is not tempo-warped and does not write into the project.

An installed plugin is not necessarily authorized. GDAW excludes reported load failures and supports manual hiding/retry. A plugin may defer its license dialog or continue loading in a demo state; GDAW does not claim a universal iLok license-status API. Authorization remains with the plugin and its licensing system. Existing session inserts and saved states are retained when a plugin becomes unavailable.

## Native and exchange formats

| Type | Purpose | Contents |
|---|---|---|
| `.gdaw` | Editable native session | Schema-versioned JSON, with media in the project folder |
| `.gdawz` | Portable audio project | ZIP containing `.gdaw` and decoded audio as float WAV |
| `.wav`, `.aif`, `.flac` | Audio delivery | Existing bounce formats |
| `.mid` | MIDI interchange | Standard MIDI File including the song tempo map |
| WAV/MIDI export folder | Move rendered tracks to another DAW | Equal-length 24-bit WAV tracks, main reference mix, MIDI, session information and JSON manifest |

Older `.scraft` and `.scraftz` sessions remain readable. Schema version 1 is preserved; changing the native extension does not invent a new audio encoding. Model serialization is defined in `crates/model/src/session.rs`, `track.rs`, `mixer.rs` and `automation.rs`. Plugins retain their format-specific state and require the corresponding installation and license to remain editable.

`file.export_project {path}` writes a portable project without renaming or cleaning the open session. It refuses missing decoded media instead of silently shipping an incomplete archive. Portable archives currently support audio projects up to 1 GiB and 10,000 entries; use a native session folder for larger projects or video. Video is not embedded in the portable audio project.

`file.export_exchange {dir}` creates a new folder. The desktop performs the render in slices with progress and cancellation; the headless command runs to completion. The default range starts at zero and includes two seconds after the content end. Explicit `start` and `end` override it. All WAVs share that origin and length. MIDI retains the original timeline; the manifest records both origins. Track WAVs capture each track's processing, fader and pan, without its send returns or main master processing. Those are represented in the main reference mix. They are rendered track outs, not a lossless reconstruction of arbitrary nonlinear bus routing. The JSON is metadata, not an editable native session with media.

```python
with GDAWClient(7801) as gdaw:
    gdaw.execute("file.export_exchange", {"dir": "/absolute/path/My Song Export"})
    print(gdaw.request("export.status"))
```

Existing export folders are refused. Intermediate output stays in a private staging folder; success publishes the completed folder. Cancellation or a normal error cleans that staging folder. A process crash can leave a staging folder, and exported audio has not been validated against hardware or other DAWs.

Native Pro Tools `.ptx`, Ableton `.als`, AAF and OMF import/export are not implemented by these files. Standard WAV/MIDI delivery should not be confused with native-session interoperability.

## MCP and further protocol details

See [control-protocol.md](control-protocol.md) and [mcp.md](mcp.md) for transport, command references, headless operation and optional filesystem confinement. Headless confinement does not apply to an unconfined desktop instance. This pass does not introduce an authenticated remote service, native DSP extension ABI or sandbox for third-party native plugins.

This implementation is a source candidate. Rust compilation, test suites, live SDK requests, real plugins, audio preview, exchange round trips and installation remain unverified for this revision.
