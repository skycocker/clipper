//! Raw-mode terminal handling. Kept trivial — the only invariant is "if we
//! enabled raw mode, we must put it back on every exit path including panic".

use anyhow::{Context, Result};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// RAII guard that puts the terminal into raw mode for its lifetime and
/// restores cooked-mode on drop.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn new() -> Result<Self> {
        enable_raw_mode().context("enable_raw_mode failed")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}
