#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyserial>=3.5"]
# ///
"""Hardware integration test for clipper's reconnect-on-disconnect.

Drives the Flipper via USB CLI (no physical button presses needed):

  1. Force the plugin into a known state: close any running app, then
     launch /ext/apps/Bluetooth/clipper.fap.
  2. Spawn `clipper` as a subprocess. Its stdin is /dev/null (so we
     trigger EOF immediately if we want, but we keep it open for this
     test); stderr is captured.
  3. Wait for "connected" in clipper's stderr.
  4. Issue `loader close` over USB (= press Back on the Flipper).
  5. Watch for "reconnecting" in clipper's stderr within RECONNECT_TIMEOUT.
  6. Kill clipper. Restore Flipper to the main menu (close any app).

Exit 0 = pass; non-zero = fail with descriptive message.

Run:
    uv run tools/test_reconnect.py
    uv run tools/test_reconnect.py --clipper-bin path/to/clipper

Prereqs: flipper plugged in, fresh build of clipper at
client/target/release/clipper (or wherever --clipper-bin points), and
the plugin .fap installed at /ext/apps/Bluetooth/clipper.fap (which
`ufbt launch` already does).
"""
from __future__ import annotations

import argparse
import os
import select
import signal
import subprocess
import sys
import time
from pathlib import Path

# Reuse the CLI wrapper next to us.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import flipper_cli  # noqa: E402

PLUGIN_PATH = "/ext/apps/Bluetooth/clipper.fap"
PLUGIN_BOOT_S = 1.5
CONNECT_TIMEOUT_S = 30.0
RECONNECT_TIMEOUT_S = 60.0  # see how long this actually takes on macOS+btleplug
CLIPPER_BIN_DEFAULT = (
    Path(__file__).resolve().parent.parent / "client" / "target" / "release" / "clipper"
)


def flipper(command: str, timeout: float = 2.0) -> bytes:
    return flipper_cli.send_command(flipper_cli.find_port(), command, timeout)


def ensure_plugin_running() -> None:
    info = flipper("loader info").decode("utf-8", "replace")
    if "CLIpper" in info:
        # Leftover from a prior run — close it cleanly so we start from a
        # known state (and free up BT so flipwire/etc. work afterward).
        print("[setup] CLIpper plugin already running, closing first")
        flipper("input send back short", timeout=2.0)
        time.sleep(0.5)
        info = flipper("loader info").decode("utf-8", "replace")
    if "No application" not in info:
        print(f"[setup] closing current app: {info.strip()}")
        flipper("input send back short", timeout=2.0)
        time.sleep(0.3)
    print("[setup] launching plugin")
    out = flipper(f"loader open {PLUGIN_PATH}", timeout=4.0).decode("utf-8", "replace")
    if "not found" in out.lower() or "error" in out.lower():
        raise SystemExit(f"[setup] loader open failed: {out!r}")
    time.sleep(PLUGIN_BOOT_S)
    info = flipper("loader info").decode("utf-8", "replace")
    if "CLIpper" not in info:
        raise SystemExit(f"[setup] plugin failed to start: {info!r}")
    print(f"[setup] plugin running: {info.strip()}")


def wait_for_in_stderr(proc: subprocess.Popen, needle: str, timeout_s: float) -> str:
    """Block until `needle` appears in proc's stderr, return accumulated text."""
    assert proc.stderr is not None
    buf = ""
    deadline = time.monotonic() + timeout_s
    fd = proc.stderr.fileno()
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise SystemExit(
                f"[test] clipper exited unexpectedly (rc={proc.returncode}) before seeing {needle!r}\n"
                f"stderr so far:\n{buf}"
            )
        ready, _, _ = select.select([fd], [], [], 0.2)
        if not ready:
            continue
        chunk = os.read(fd, 4096).decode("utf-8", "replace")
        if not chunk:
            continue
        buf += chunk
        sys.stderr.write(chunk)
        sys.stderr.flush()
        if needle in buf:
            return buf
    raise SystemExit(
        f"[test] timeout: did not see {needle!r} within {timeout_s}s.\nstderr so far:\n{buf}"
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--clipper-bin", default=str(CLIPPER_BIN_DEFAULT))
    args = ap.parse_args()

    binp = Path(args.clipper_bin)
    if not binp.is_file():
        raise SystemExit(f"clipper binary not found at {binp} — run `cargo build --release`")

    ensure_plugin_running()

    print(f"[test] starting {binp}")
    env = dict(os.environ)
    env["CLIPPER_EVENT_DEBUG"] = "1"  # surface adapter events so we can see them
    # stdin must be a PIPE (kept open with nothing written) so clipper
    # doesn't see EOF and exit via SessionExit::StdinClosed. DEVNULL would
    # close it immediately.
    proc = subprocess.Popen(
        [str(binp)],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        env=env,
    )

    try:
        wait_for_in_stderr(proc, "connected", CONNECT_TIMEOUT_S)
        t0 = time.monotonic()
        print(f"\n[test t={t0:.2f}] clipper connected — sending input send back short")
        flipper("input send back short", timeout=2.0)
        print(f"[test t={time.monotonic()-t0:.2f}s] trigger sent, watching for 'reconnecting'")
        wait_for_in_stderr(proc, "reconnecting", RECONNECT_TIMEOUT_S)
        print(f"[test t={time.monotonic()-t0:.2f}s] reconnect detected")
        print("\n[test] PASS — reconnect was triggered after disconnect")
        return 0
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
        # Best-effort tidy up so a follow-up run starts from a known state.
        try:
            flipper("input send back short", timeout=2.0)
        except Exception as e:
            print(f"[teardown] loader close failed: {e}", file=sys.stderr)


if __name__ == "__main__":
    sys.exit(main())
