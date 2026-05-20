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

static const GapConfig clipper_gap_template = {
    .adv_service =
        {
            .UUID_Type = UUID_TYPE_16,
            .Service_UUID_16 = 0x3081, /* one off from stock serial's 0x3080 to avoid cache confusion */
        },
    .appearance_char = 0x8600,
    /* BleServiceSerial characteristics require AUTHEN_READ/WRITE permissions,
     * so we have to do real BLE bonding. macOS will pop the numeric-comparison
     * pairing dialog on first connect; once bonded, future runs are silent. */
    .bonding_mode = true,
    .pairing_method = GapPairingPinCodeShow,
    .conn_param = {
        .conn_int_min = CONNECTION_INTERVAL_MIN,
        .conn_int_max = CONNECTION_INTERVAL_MAX,
        .slave_latency = 0,
        .supervisor_timeout = 0,
    },
};

static void clipper_serial_profile_get_config(GapConfig* config, FuriHalBleProfileParams params) {
    UNUSED(params);
    furi_check(config);

    memcpy(config, &clipper_gap_template, sizeof(GapConfig));
    memcpy(config->mac_address, furi_hal_version_get_ble_mac(), sizeof(config->mac_address));
    /* Distinguish from stock serial profile's MAC so bond storage entries don't collide. */
    config->mac_address[2] ^= 0x42;

    strlcpy(config->adv_name, "CLIpper", sizeof(config->adv_name));
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
