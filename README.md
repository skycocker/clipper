# clipper

Interactive Flipper Zero CLI shell over Bluetooth — like `screen /dev/cu.usbmodem*`,
but cordless.

Two pieces:

- **`plugin/`** — a Flipper `.fap` that, when launched on the device, exposes a
  Nordic-UART-style GATT service and bridges it to a real Flipper CLI session.
- **`client/`** — a host-side Rust CLI (`clipper`) that connects to that GATT
  service and gives you a normal terminal: type commands, see streaming output,
  `^C` to interrupt, `^D` to exit.

## Status

Pre-alpha. Project scaffolding only.

## Platforms

| OS | Status |
|---|---|
| macOS (Apple Silicon) | Primary target. Tested daily. |
| Linux | Supported. Tested before each release. First-time BLE pair via `bluetoothctl`. |
| Windows 11 | Experimental. CI builds the binary; no hand-tested validation yet. Reports welcome. |

## Quick start

*(Coming once the alpha builds.)*

## License

MIT — see [LICENSE](LICENSE).
