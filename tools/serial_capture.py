#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyserial>=3.5"]
# ///
"""Continuously capture the Flipper's USB serial console to a file.

Survives the USB renumeration that a firmware crash/reboot causes by
reopening the port whenever it drops. Flipper prints its panic reason
("furi_check failed", "NULL pointer", file:line, PC/LR) to the console
right before rebooting, and OFW/Momentum re-prints the saved crash
string in the boot banner — so capturing across the crash window gets
us the actual fault.

Usage:
    uv run tools/serial_capture.py [--out FILE] [--seconds N]

Prints everything it captures to stdout too (with [reopened] markers).
"""
import argparse
import glob
import sys
import time

import serial


def find_port() -> str | None:
    c = sorted(glob.glob("/dev/cu.usbmodemflip_*"))
    return c[0] if c else None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="/tmp/clipper_serial.log")
    ap.add_argument("--seconds", type=float, default=60.0)
    args = ap.parse_args()

    deadline = time.monotonic() + args.seconds
    logf = open(args.out, "wb")
    print(f"[capture] logging to {args.out} for {args.seconds:.0f}s", flush=True)

    ser = None
    while time.monotonic() < deadline:
        if ser is None:
            port = find_port()
            if port is None:
                time.sleep(0.1)
                continue
            try:
                ser = serial.Serial(port, 115200, timeout=0.1)
                msg = f"\n[capture] opened {port} @ {time.monotonic():.2f}\n"
                sys.stdout.write(msg)
                sys.stdout.flush()
                logf.write(msg.encode())
            except Exception as e:
                sys.stdout.write(f"[capture] open failed: {e}\n")
                sys.stdout.flush()
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
            msg = f"\n[capture] port dropped (likely crash/reboot) @ {time.monotonic():.2f}\n"
            sys.stdout.write(msg)
            sys.stdout.flush()
            logf.write(msg.encode())
            try:
                ser.close()
            except Exception:
                pass
            ser = None
            time.sleep(0.1)

    if ser is not None:
        ser.close()
    logf.close()
    print("\n[capture] done", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
