#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["bleak"]
# ///
"""Spike validation: confirms our .fap-defined BLE profile is reachable.

What this checks:
1. The Flipper advertises as 'Clipper' (set in clipper_test_profile.c get_gap_config).
2. We can connect via CoreBluetooth/bleak.
3. The custom service (12345678-cccc-eeee-0001-deadbeef0000) is present.
4. Reading the test characteristic returns the bytes b"hello".

Prereqs:
- `ufbt launch_app` (or sideload + launch) the clipper.fap on the Flipper.
- The plugin's main screen says "BLE: Active".
- Nothing else (mobile app, qFlipper) is connected to the Flipper.

Run:  `uv run tools/spike_smoke.py`
Exit codes: 0 = pass, 1 = no device, 2 = no service, 3 = wrong payload.
"""
import asyncio
import sys

from bleak import BleakClient, BleakScanner

CLIPPER_SVC_UUID = "12345678-cccc-eeee-0001-deadbeef0000"
CLIPPER_CHAR_UUID = "12345678-cccc-eeee-0001-deadbeef0001"
# 16-bit GAP advertising UUID (set in clipper_test_profile.c get_gap_config).
# Expanded to full 128-bit form via the Bluetooth Base UUID for matching.
CLIPPER_ADV_UUID_16 = "0000c11f-0000-1000-8000-00805f9b34fb"
EXPECTED = b"hello"
SCAN_SECONDS = 12.0


async def find_clipper(timeout: float):
    print(f"Scanning {timeout:.0f}s for the Clipper advertising UUID ({CLIPPER_ADV_UUID_16})...")
    found: dict = {}

    def cb(device, adv):
        svcs = [s.lower() for s in (adv.service_uuids or [])]
        name = adv.local_name or device.name or ""
        if CLIPPER_ADV_UUID_16 in svcs or name.lower().startswith("clipper"):
            found[device.address] = (device, adv)

    async with BleakScanner(detection_callback=cb):
        await asyncio.sleep(timeout)
    return found


async def main() -> int:
    found = await find_clipper(SCAN_SECONDS)
    if not found:
        print("FAIL: no Clipper advertisement seen.")
        print("  - Is the plugin actually running? (Should show 'BLE: Active' on the Flipper screen.)")
        print("  - Is anything else connected to the Flipper? (Close iOS/qFlipper apps.)")
        return 1

    for addr, (device, adv) in found.items():
        print(f"  found: {addr}  rssi={adv.rssi}  name={adv.local_name!r}")

    addr, (device, _) = next(iter(found.items()))
    print(f"\nConnecting to {addr}...")
    async with BleakClient(device, timeout=20.0) as client:
        print(f"  connected: {client.is_connected}")

        services = list(client.services)
        print(f"\nServices found ({len(services)}):")
        clipper_svc = None
        for svc in services:
            marker = "  <-- OUR SERVICE" if svc.uuid.lower() == CLIPPER_SVC_UUID else ""
            print(f"  {svc.uuid}{marker}")
            for ch in svc.characteristics:
                print(f"    char {ch.uuid}  [{','.join(ch.properties)}]")
            if svc.uuid.lower() == CLIPPER_SVC_UUID:
                clipper_svc = svc

        if clipper_svc is None:
            print("FAIL: custom service UUID not present after connect.")
            return 2

        print(f"\nReading {CLIPPER_CHAR_UUID}...")
        value = await client.read_gatt_char(CLIPPER_CHAR_UUID)
        print(f"  got: {value!r}  (expected: {EXPECTED!r})")
        if value != EXPECTED:
            print("FAIL: payload mismatch.")
            return 3

    print("\nPASS: .fap-defined profile is fully working over BLE on macOS.")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
