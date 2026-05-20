//! Raw-mode terminal handling. Kept trivial — the only invariant is "if we
//! enabled raw mode, we must put it back on every exit path including panic".
//!
//! If stdin is not a TTY (test harness, subprocess pipes), we skip raw mode
//! entirely. Same code path runs; the terminal just stays in cooked mode,
//! which is the right behavior for piped input anyway.

use std::io::IsTerminal;

use anyhow::{Context, Result};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// RAII guard that puts the terminal into raw mode for its lifetime and
/// restores cooked-mode on drop. No-op when stdin isn't a TTY.
pub struct RawModeGuard {
    enabled: bool,
}

impl RawModeGuard {
    pub fn new() -> Result<Self> {
        if !std::io::stdin().is_terminal() {
            return Ok(Self { enabled: false });
        }
        enable_raw_mode().context("enable_raw_mode failed")?;
        Ok(Self { enabled: true })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            let _ = disable_raw_mode();
        }
    }
}
