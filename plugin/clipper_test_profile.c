#include "clipper_test_profile.h"

#include <furi.h>
#include <gap.h>
#include <furi_hal_version.h>
#include <furi_ble/gatt.h>
#include <furi_ble/profile_interface.h>
#include <ble/core/ble_defs.h>

/* Service UUID:        12345678-cccc-eeee-0001-deadbeef0000  (LE bytes)
 * Characteristic UUID: 12345678-cccc-eeee-0001-deadbeef0001  (LE bytes) */
static const uint8_t clipper_test_service_uuid[16] = {
    0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x01, 0x00,
    0xee, 0xee, 0xcc, 0xcc, 0x78, 0x56, 0x34, 0x12,
};
static const uint8_t clipper_test_char_uuid[16] = {
    0x01, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x01, 0x00,
    0xee, 0xee, 0xcc, 0xcc, 0x78, 0x56, 0x34, 0x12,
};

static const uint8_t hello_payload[] = "hello";
#define HELLO_LEN (sizeof(hello_payload) - 1) /* strip NUL */

static const BleGattCharacteristicParams clipper_test_char_params = {
    .name = "Hello",
    .data_prop_type = FlipperGattCharacteristicDataFixed,
    .data.fixed.ptr = hello_payload,
    .data.fixed.length = HELLO_LEN,
    .uuid_type = UUID_TYPE_128,
    .char_properties = CHAR_PROP_READ,
    .security_permissions = ATTR_PERMISSION_NONE,
    .gatt_evt_mask = GATT_DONT_NOTIFY_EVENTS,
    .is_variable = CHAR_VALUE_LEN_CONSTANT,
};

typedef struct {
    FuriHalBleProfileBase base;
    uint16_t svc_handle;
    BleGattCharacteristicInstance char_instance;
} ClipperTestProfile;

_Static_assert(offsetof(ClipperTestProfile, base) == 0, "Wrong layout");

static FuriHalBleProfileBase* clipper_test_profile_start(FuriHalBleProfileParams params) {
    UNUSED(params);

    ClipperTestProfile* profile = malloc(sizeof(ClipperTestProfile));
    profile->base.config = clipper_test_profile;

    Service_UUID_t svc_uuid;
    memcpy(svc_uuid.Service_UUID_128, clipper_test_service_uuid, 16);
    if(!ble_gatt_service_add(
           UUID_TYPE_128, &svc_uuid, PRIMARY_SERVICE, 4, &profile->svc_handle)) {
        free(profile);
        return NULL;
    }

    BleGattCharacteristicParams char_params = clipper_test_char_params;
    memcpy(char_params.uuid.Char_UUID_128, clipper_test_char_uuid, 16);
    ble_gatt_characteristic_init(
        profile->svc_handle, &char_params, &profile->char_instance);

    /* ble_gatt_characteristic_init only registers the characteristic and
     * allocates an attribute record of the given length, filled with zeros.
     * To populate it with our fixed data, we have to call update(NULL) once,
     * which copies from data.fixed.ptr. */
    ble_gatt_characteristic_update(profile->svc_handle, &profile->char_instance, NULL);

    return &profile->base;
}

static void clipper_test_profile_stop(FuriHalBleProfileBase* base) {
    furi_check(base);
    furi_check(base->config == clipper_test_profile);

    ClipperTestProfile* profile = (ClipperTestProfile*)base;
    ble_gatt_characteristic_delete(profile->svc_handle, &profile->char_instance);
    ble_gatt_service_delete(profile->svc_handle);
    free(profile);
}

#define CONNECTION_INTERVAL_MIN (0x06)
#define CONNECTION_INTERVAL_MAX (0x24)

static const GapConfig clipper_test_gap_template = {
    .adv_service =
        {
            .UUID_Type = UUID_TYPE_16,
            .Service_UUID_16 = 0xC11F, /* arbitrary 16-bit ad UUID for discoverability */
        },
    .appearance_char = 0x0000,
    .bonding_mode = false, /* spike: no bonding required, easier to test */
    .pairing_method = GapPairingNone,
    .conn_param = {
        .conn_int_min = CONNECTION_INTERVAL_MIN,
        .conn_int_max = CONNECTION_INTERVAL_MAX,
        .slave_latency = 0,
        .supervisor_timeout = 0,
    },
};

static void clipper_test_profile_get_config(GapConfig* config, FuriHalBleProfileParams params) {
    UNUSED(params);
    furi_check(config);

    memcpy(config, &clipper_test_gap_template, sizeof(GapConfig));
    memcpy(config->mac_address, furi_hal_version_get_ble_mac(), sizeof(config->mac_address));
    /* Offset MAC so we don't collide with the default serial profile's bond entry. */
    config->mac_address[2] ^= 0x42;

    strlcpy(config->adv_name, "Clipper", sizeof(config->adv_name));
}

static const FuriHalBleProfileTemplate clipper_test_profile_callbacks = {
    .start = clipper_test_profile_start,
    .stop = clipper_test_profile_stop,
    .get_gap_config = clipper_test_profile_get_config,
};

const FuriHalBleProfileTemplate* clipper_test_profile = &clipper_test_profile_callbacks;
