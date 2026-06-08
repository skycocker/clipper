# clipper

**Interactive Flipper Zero CLI shell over Bluetooth — like `screen /dev/cu.usbmodem*`, but cordless.**

The Flipper has a great CLI accessible over its USB CDC interface (`storage`,
`subghz`, `nfc`, `gpio`, `bt`, `ps`, etc.). Over Bluetooth, the stock firmware
exposes the *same serial endpoint* — but routes every byte to a protobuf-RPC
subsystem instead of the CLI shell. So no interactive shell over BLE without
something on both ends.

`clipper` is that something. A small Flipper `.fap` plugin reroutes the
BLE serial endpoint to a real `cli_shell_alloc()` session, and a host-side
Rust binary opens a raw-mode TTY that pipes to/from it.

```
$ clipper
clipper: scanning 12s for "CLIpper"...
clipper: match name="CLIpper" svc=yes rssi=Some(-42)
clipper: connecting...
clipper: connected — type to send, Ctrl+] (or Ctrl+\, Ctrl+D) to exit.

CLIpper :: BLE CLI shell

>: ps
Name                           Stack
DesktopSrv                     2048
GuiSrv                         2048
...
>:
```

## Project layout

| Dir | What |
|---|---|
| `plugin/` | The Flipper `.fap`. Registers its own `FuriHalBleProfileTemplate` reusing the stock `BleServiceSerial` shape so existing clients (any "Flipper serial over BLE" tool) work too. |
| `client/` | The Rust binary. `btleplug` + `tokio` + `crossterm`. |
| `tools/` | Diagnostic Python scripts: `scan.py` (BLE scan), `flipper_cli.py` (drive Flipper over USB), `test_reconnect.py` (hardware integration test). |

## Platforms

| OS | Status |
|---|---|
| macOS (Apple Silicon) | Primary target. Daily-used. |
| Linux | Supported. Tested before each release. First-time BLE pair via `bluetoothctl pair <addr>`. |
| Windows 11 | Experimental. CI builds the binary; no hand-tested validation yet. Reports welcome. |

## Quick start

Until prebuilt binaries are published, build from source.

**Prereqs:**
- Rust (any stable >= 1.75)
- Python 3.12 + [uv](https://docs.astral.sh/uv/) — only for the diagnostic scripts
- [`ufbt`](https://github.com/flipperdevices/flipperzero-ufbt) — only to build/flash the plugin
- A Flipper Zero (any firmware that ships the modern `cli_shell_alloc` API — Momentum tested)

**1. Build and flash the plugin** (USB cable):
```
cd plugin
ufbt launch     # builds, copies to /ext/apps/Bluetooth/clipper.fap, and launches it
```

The plugin's screen should show "CLIpper / BLE CLI: ready / Back to exit".

**2. Build and run the client:**
```
cd client
cargo run --release
```

First connect pops a system pairing dialog on the Mac and a 6-digit numeric
comparison prompt on the Flipper. Confirm on both. Once bonded, subsequent
runs are silent.

## Usage

```
clipper [<name-substring>]
```

- No arg: match the default `CLIpper` name OR our `0x3081` advertising service UUID.
- With arg: match any device whose advertised name contains the substring (case-insensitive). Useful if you've renamed the plugin.

**Exit keys** (any of these work; we accept several because terminal emulators
sometimes intercept individual control bytes):
- `Ctrl+]` — telnet-style escape
- `Ctrl+\` — file separator
- `Ctrl+D` — EOT

`Ctrl+C` is forwarded to the Flipper as 0x03 so you can interrupt a running
remote command without killing the local client.

The shell shares the device's main CLI command registry **and** its external
command config, so both built-in commands (`storage`, `ps`, `gpio`, …) and
external `.fal` commands work — including the interactive sub-shells. For
example, to drive NFC over Bluetooth:

```
>: nfc
[nfc]>: scanner      # or: emulate / apdu / dump / raw / mfu / field
```

This is the intended way to debug NFC interactions cordlessly: run clipper as
the foreground app and drive `nfc` from the BLE shell. (Only one Flipper app
runs at a time, so this *replaces* the dedicated NFC app rather than running
alongside it.)

**Environment variables:**
- `CLIPPER_SCAN_DEBUG=1` — dump every BLE peripheral seen during scan (useful when troubleshooting "device not found").

## Diagnostics / hardware tests (`tools/`)

- `scan.py` — list nearby BLE devices.
- `flipper_cli.py "<cmd>"` — run a command on the Flipper's **USB** CLI (e.g. `"loader open …"`, `"input send back short"`); used to drive the device in tests without touching it.
- `serial_capture.py` / `log_capture.py` — capture the Flipper's USB console / `log trace` stream across a crash+reboot (the reboot renumerates USB; these reopen it). Invaluable for locating faults without an SWD probe.
- `spike_smoke.py` — bleak-based connect + `help` round-trip check.
- `test_reconnect.py` — end-to-end reconnect integration test.

## Known limitations

- **Slow disconnect detection on macOS.** When the plugin exits without
  sending a graceful link-layer termination (which happens any time the
  user presses Back), macOS / CoreBluetooth / `btleplug` can take many
  seconds to surface the disconnect. The reconnect loop kicks in once it
  does. Linux/BlueZ does not have this latency.
- **Advertised name truncates by one byte on stock firmware.** We work
  around it by also matching by service UUID `0x3081`.

## Development

```
# rust unit tests + clippy
cd client
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# plugin build (no-flash)
cd plugin
ufbt

# hardware integration tests (needs Flipper connected via USB)
uv run tools/test_reconnect.py
```

CI runs `cargo fmt`/`clippy`/`test` on the `{ubuntu, macos, windows}-latest` runner matrix and builds the plugin on Ubuntu via `ufbt`.

## License

MIT — see [LICENSE](LICENSE).
