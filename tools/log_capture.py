#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyserial>=3.5"]
# ///
"""Stream the Flipper's `log trace` over USB to a file, surviving reboots.

Opens the CLI, sets max verbosity, issues `log` (live stream), and dumps
everything to a file + stdout. When the device crashes, the last lines
before the USB drop are the last subsystems that executed — which tells
us where the HardFault came from even without a debug probe.

Usage: uv run tools/log_capture.py [--out FILE] [--seconds N]
"""
import argparse
import glob
import sys
import time

import serial


def find_port():
    c = sorted(glob.glob("/dev/cu.usbmodemflip_*"))
    return c[0] if c else None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="/tmp/clipper_log.log")
    ap.add_argument("--seconds", type=float, default=40.0)
    args = ap.parse_args()

    deadline = time.monotonic() + args.seconds
    logf = open(args.out, "wb")
    ser = None
    started_stream = False

    while time.monotonic() < deadline:
        if ser is None:
            port = find_port()
            if not port:
                time.sleep(0.1)
                continue
            try:
                ser = serial.Serial(port, 115200, timeout=0.1)
                time.sleep(0.3)
                ser.reset_input_buffer()
                # Max verbosity, then start the live log stream.
                ser.write(b"log trace\r")
                ser.flush()
                started_stream = True
                m = f"\n[logcap] streaming `log trace` from {port}\n"
                sys.stdout.write(m)
                sys.stdout.flush()
                logf.write(m.encode())
            except Exception as e:
                sys.stdout.write(f"[logcap] open failed: {e}\n")
                sys.stdout.flush()
                ser = None
                time.sleep(0.2)
                continue
        try:
            data = ser.read(4096)
            if data:
                logf.write(data)
                logf.flush()
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
        except (serial.SerialException, OSError):
            m = f"\n[logcap] *** PORT DROPPED (crash/reboot) @ {time.monotonic():.2f} ***\n"
            sys.stdout.write(m)
            sys.stdout.flush()
            logf.write(m.encode())
            try:
                ser.close()
            except Exception:
                pass
            ser = None
            started_stream = False
            time.sleep(0.1)

    if ser:
        ser.close()
    logf.close()
    sys.stdout.write("\n[logcap] done\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
