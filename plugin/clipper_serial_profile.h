#pragma once

#include <furi_ble/profile_interface.h>
#include <services/serial_service.h>

#ifdef __cplusplus
extern "C" {
#endif

/* clipper_serial_profile: a Flipper BLE profile that exposes the standard
 * BleServiceSerial (8fe5b3d5-2e7f-4a98-2a48-7acc60fe0000 + characteristics)
 * but under our own profile-template identity. The bt service's auto-RPC
 * routing checks profile identity via furi_hal_bt_check_profile_type(...,
 * ble_profile_serial), which fails for us — so we own the serial
 * service's event callback instead. */
extern const FuriHalBleProfileTemplate* clipper_serial_profile;

/* Access the embedded BleServiceSerial so the app can set callbacks and
 * push TX data. Asserts profile identity. */
BleServiceSerial* clipper_serial_profile_get_service(FuriHalBleProfileBase* profile);

#ifdef __cplusplus
}
#endif
