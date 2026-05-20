#pragma once

#include <furi_ble/profile_interface.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Spike profile: registers a single GATT service with one read-only
 * characteristic that returns the fixed bytes "hello". Used to prove the
 * .fap-defined profile path works end-to-end on hardware. */
extern const FuriHalBleProfileTemplate* clipper_test_profile;

#ifdef __cplusplus
}
#endif
