#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["bleak"]
# ///
"""End-to-end smoke test for the Clipper BLE CLI bridge.

What this does:
1. Scan for a Clipper advertisement (matches GAP service UUID 0x3081 OR
   advertised name "Clipper").
2. Connect via bleak. First time triggers a macOS pairing dialog — confirm
   the 6-digit code on both Mac and Flipper.
3. Subscribe to indications on the Flipper TX characteristic.
4. Write a CLI command to the RX characteristic (`help\\r`).
5. Collect ~3 seconds of output, print it, and assert it's non-trivial.

Run:  uv run tools/spike_smoke.py
Prereqs: clipper.fap is running on the Flipper (screen says
"BLE CLI: ready"), nothing else connected to the device.
"""
import asyncio
import sys

from bleak import BleakClient, BleakScanner

# Standard Flipper serial service UUIDs (our profile reuses BleServiceSerial).
SERIAL_SVC_UUID = "8fe5b3d5-2e7f-4a98-2a48-7acc60fe0000"
SERIAL_RX_UUID = "19ed82ae-ed21-4c9d-4145-228e62fe0000"  # host -> Flipper (write)
SERIAL_TX_UUID = "19ed82ae-ed21-4c9d-4145-228e61fe0000"  # Flipper -> host (indicate)

# 16-bit GAP advertising UUID we set in clipper_serial_profile.c. Expanded to
# 128-bit via the Bluetooth Base UUID for matching against adv data.
CLIPPER_ADV_UUID_16 = "00003081-0000-1000-8000-00805f9b34fb"

SCAN_SECONDS = 12.0
COLLECT_SECONDS = 3.0
COMMAND = b"help\r"


async def find_clipper(timeout: float):
    print(f"Scanning {timeout:.0f}s for a Clipper advertisement...")
    found: dict = {}

    def cb(device, adv):
        name = adv.local_name or device.name or ""
        svcs = [s.lower() for s in (adv.service_uuids or [])]
        if name.lower().startswith("clipper") or CLIPPER_ADV_UUID_16 in svcs:
            found[device.address] = (device, adv)

    async with BleakScanner(detection_callback=cb):
        await asyncio.sleep(timeout)
    return found


async def main() -> int:
    found = await find_clipper(SCAN_SECONDS)
    if not found:
        print("FAIL: no Clipper advertisement seen. Is the plugin running?")
        return 1

    for addr, (_, adv) in found.items():
        print(f"  found: {addr}  rssi={adv.rssi}  name={adv.local_name!r}")

    addr, (device, _) = next(iter(found.items()))
    print(f"\nConnecting to {addr}...")
    print("  (First connect may pop a pairing dialog. Confirm the matching 6-digit code on Mac AND Flipper.)")

    received: bytearray = bytearray()

    def on_indication(_char, data: bytearray):
        received.extend(data)
        sys.stdout.write(data.decode("utf-8", "replace"))
        sys.stdout.flush()

    async with BleakClient(device, timeout=30.0) as client:
        print(f"  connected: {client.is_connected}\n")

        services = list(client.services)
        svc_uuids = [s.uuid.lower() for s in services]
        if SERIAL_SVC_UUID not in svc_uuids:
            print(f"FAIL: serial service {SERIAL_SVC_UUID} not present.")
            print(f"  Available: {svc_uuids}")
            return 2

        print(f"Subscribing to TX ({SERIAL_TX_UUID})...")
        await client.start_notify(SERIAL_TX_UUID, on_indication)
        await asyncio.sleep(0.5)  # let initial prompt arrive

        print(f"Writing {COMMAND!r} to RX ({SERIAL_RX_UUID})...\n")
        print("--- BLE CLI output below ---")
        await client.write_gatt_char(SERIAL_RX_UUID, COMMAND, response=True)

        await asyncio.sleep(COLLECT_SECONDS)
        await client.stop_notify(SERIAL_TX_UUID)

    print("\n--- end output ---\n")
    print(f"Collected {len(received)} bytes from TX.")
    if len(received) < 5:
        print("FAIL: not enough output. Bridge probably isn't wired correctly.")
        return 3

    text = received.decode("utf-8", "replace").lower()
    # Loose assertion — `help` typically prints a list of commands, and the
    # Flipper CLI prompt is `>:`. Look for either as a sanity check.
    if "help" not in text and "available" not in text and ">:" not in text:
        print("FAIL: output doesn't look like CLI response.")
        return 4

    print("PASS: BLE CLI bridge round-tripped a real CLI command.")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
