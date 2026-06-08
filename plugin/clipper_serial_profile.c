#include "clipper_serial_profile.h"

#include <furi.h>
#include <furi_hal_version.h>
#include <gap.h>
#include <ble/core/ble_defs.h>
#include <services/dev_info_service.h>
#include <services/battery_service.h>

typedef struct {
    FuriHalBleProfileBase base;
    BleServiceDevInfo* dev_info;
    BleServiceBattery* battery;
    BleServiceSerial* serial;
} ClipperSerialProfile;

_Static_assert(offsetof(ClipperSerialProfile, base) == 0, "Wrong layout");

static FuriHalBleProfileBase* clipper_serial_profile_start(FuriHalBleProfileParams params) {
    UNUSED(params);

    ClipperSerialProfile* p = malloc(sizeof(ClipperSerialProfile));
    p->base.config = clipper_serial_profile;
    p->dev_info = ble_svc_dev_info_start();
    p->battery = ble_svc_battery_start(true);
    p->serial = ble_svc_serial_start();
    return &p->base;
}

static void clipper_serial_profile_stop(FuriHalBleProfileBase* base) {
    furi_check(base);
    furi_check(base->config == clipper_serial_profile);

    ClipperSerialProfile* p = (ClipperSerialProfile*)base;
    ble_svc_battery_stop(p->battery);
    ble_svc_dev_info_stop(p->dev_info);
    ble_svc_serial_stop(p->serial);
    free(p);
}

#define CONNECTION_INTERVAL_MIN (0x06)
#define CONNECTION_INTERVAL_MAX (0x24)
/* Match the stock serial profile's connection parameters exactly. We tried
 * 200 (2s) here to speed up Mac-side disconnect detection, but it didn't
 * actually help (macOS overrides peripheral-requested params anyway) and
 * is suspected of being one of several inputs into the BLE pairing
 * crashes we hit during testing. Back to firmware default. */
#define CONNECTION_SUPERVISOR_TIMEOUT (0)

static const GapConfig clipper_gap_template = {
    .adv_service =
        {
            .UUID_Type = UUID_TYPE_16,
            .Service_UUID_16 = 0x3081, /* one off from stock serial's 0x3080 to avoid cache confusion */
        },
    .appearance_char = 0x8600,
    /* BleServiceSerial characteristics require AUTHEN_READ/WRITE permissions,
     * so we have to do real BLE bonding (MITM-protected). Use VerifyYesNo
     * (numeric comparison) like the HID profile — both sides show the same
     * 6-digit number and the user confirms. This is a nicer UX than
     * PinCodeShow (which makes the host TYPE a code shown on the Flipper)
     * and matches the rock-solid HID pairing path. */
    .bonding_mode = true,
    .pairing_method = GapPairingPinCodeVerifyYesNo,
    .conn_param = {
        .conn_int_min = CONNECTION_INTERVAL_MIN,
        .conn_int_max = CONNECTION_INTERVAL_MAX,
        .slave_latency = 0,
        .supervisor_timeout = CONNECTION_SUPERVISOR_TIMEOUT,
    },
};

static void clipper_serial_profile_get_config(GapConfig* config, FuriHalBleProfileParams params) {
    UNUSED(params);
    furi_check(config);

    memcpy(config, &clipper_gap_template, sizeof(GapConfig));
    memcpy(config->mac_address, furi_hal_version_get_ble_mac(), sizeof(config->mac_address));
    /* Offset the MAC from the device's stock serial-profile address so our
     * bonds live at a distinct BLE address. NOTE: 0x42 on byte[2] was used by
     * earlier builds; macOS cached a (crashed, half-formed) bond at that
     * address under the name "LIpper" and would not release it through the UI,
     * jamming every later pair with SMP "key missing" (status 3). Moving to a
     * fresh offset makes the host see a brand-new peer and pair cleanly. If a
     * future bond ever gets wedged again on the host side, bump this. */
    config->mac_address[2] ^= 0x37;

    /* gap.c stores adv_name as `<AD_TYPE_COMPLETE_LOCAL_NAME> <name>` with
     * the type byte at index 0 and the readable name starting at index 1.
     * Skipping the type byte (using just strlcpy(adv_name, "CLIpper", ...))
     * makes the first character of the name be interpreted as the AD type,
     * which is why we previously saw "LIpper" on the wire.
     * AD_TYPE_COMPLETE_LOCAL_NAME = 0x09 per Bluetooth Core spec. */
    config->adv_name[0] = (char)0x09;
    strlcpy(config->adv_name + 1, "CLIpper", sizeof(config->adv_name) - 1);
}

static const FuriHalBleProfileTemplate clipper_serial_profile_callbacks = {
    .start = clipper_serial_profile_start,
    .stop = clipper_serial_profile_stop,
    .get_gap_config = clipper_serial_profile_get_config,
};

const FuriHalBleProfileTemplate* clipper_serial_profile = &clipper_serial_profile_callbacks;

BleServiceSerial* clipper_serial_profile_get_service(FuriHalBleProfileBase* profile) {
    furi_check(profile);
    furi_check(profile->config == clipper_serial_profile);
    return ((ClipperSerialProfile*)profile)->serial;
}
