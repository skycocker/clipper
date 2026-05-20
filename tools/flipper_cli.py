#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyserial>=3.5"]
# ///
"""Send commands to the Flipper's USB CLI and capture the response.

The Flipper firmware exposes a text-mode CLI over its USB CDC interface
that, among other things, supports `loader open` / `loader close` — i.e.
launching and closing apps without physical button presses. Handy for
hardware integration tests.

Usage:
    uv run tools/flipper_cli.py "loader close"
    uv run tools/flipper_cli.py "loader open /ext/apps/Bluetooth/clipper.fap"
    uv run tools/flipper_cli.py "bt info"

Auto-detects /dev/cu.usbmodemflip_* — override with --port.

Exits 0 if the command was sent (regardless of what the Flipper said).
Prints captured output to stdout.
"""
import argparse
import glob
import sys
import time

import serial

PROMPT = b">: "
LF_PROMPT = b"\r\n>: "
DEFAULT_READ_TIMEOUT_S = 1.5


def find_port() -> str:
    candidates = sorted(glob.glob("/dev/cu.usbmodemflip_*"))
    if not candidates:
        raise SystemExit("error: no /dev/cu.usbmodemflip_* found — is the Flipper plugged in?")
    if len(candidates) > 1:
        print(f"warning: multiple Flippers found, picking {candidates[0]}", file=sys.stderr)
    return candidates[0]


def read_until_prompt(ser: serial.Serial, timeout_s: float) -> bytes:
    """Read until the CLI prompt appears or timeout expires."""
    buf = bytearray()
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        chunk = ser.read(1)
        if not chunk:
            continue
        buf.extend(chunk)
        if buf.endswith(PROMPT):
            break
    return bytes(buf)


def send_command(port: str, command: str, response_timeout_s: float) -> bytes:
    """Open serial, drain banner + any lingering prompts, send command, read response."""
    with serial.Serial(port, baudrate=115200, timeout=0.05) as ser:
        # Wake the CLI and drain. Opening the port triggers a DTR-driven
        # banner; sending CR may trigger an extra prompt. We don't try to
        # parse what comes back — just slurp until idle, then flush.
        ser.write(b"\r")
        ser.flush()
        time.sleep(0.4)
        ser.reset_input_buffer()

        cmd_bytes = command.encode("utf-8") + b"\r"
        ser.write(cmd_bytes)
        ser.flush()
        response = read_until_prompt(ser, timeout_s=response_timeout_s)

        # Strip the echoed command from the start of the response if present.
        if response.startswith(cmd_bytes):
            response = response[len(cmd_bytes):]
        # Strip trailing prompt for cleanliness.
        if response.endswith(LF_PROMPT):
            response = response[: -len(LF_PROMPT)]
        elif response.endswith(PROMPT):
            response = response[: -len(PROMPT)]
        return response


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("command", help="CLI command to send, e.g. 'loader close'")
    ap.add_argument("--port", default=None, help="Serial port (default: auto-detect)")
    ap.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_READ_TIMEOUT_S,
        help=f"Seconds to wait for response (default {DEFAULT_READ_TIMEOUT_S})",
    )
    args = ap.parse_args()
    port = args.port or find_port()

    response = send_command(port, args.command, args.timeout)
    sys.stdout.buffer.write(response)
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
