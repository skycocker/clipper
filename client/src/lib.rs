//! clipper — interactive Flipper Zero CLI shell over Bluetooth.
//!
//! The binary is thin glue; this library hosts the testable pieces:
//!
//! - [`ble`] — BLE abstraction (the [`ble::FlipperWriter`] trait) and the
//!   real `btleplug`-backed implementation.
//! - [`session`] — the bidirectional pipe between local stdio and the
//!   Flipper, decoupled from concrete IO so it can be exercised with mock
//!   streams.
//! - [`terminal`] — RAII raw-mode guard.

pub mod ble;
pub mod reconnect;
pub mod session;
pub mod terminal;
