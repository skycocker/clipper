//! clipper — interactive Flipper Zero CLI shell over Bluetooth.
//!
//! Thin glue around [`clipper::ble`], [`clipper::session`], and
//! [`clipper::terminal`]. See library docs for the moving parts.

use std::env;
use std::time::Duration;

use anyhow::Result;

use clipper::ble;
use clipper::session::run_session;
use clipper::terminal::RawModeGuard;

const DEFAULT_NAME_FILTER: &str = "CLIpper";
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    let name_filter = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_NAME_FILTER.to_string());
    let debug = env::var("CLIPPER_SCAN_DEBUG").is_ok();

    let (writer, notifications) =
        ble::connect(&name_filter, SCAN_TIMEOUT, CONNECT_TIMEOUT, debug).await?;

    eprintln!("clipper: connected — type to send, Ctrl+] to exit.\n");

    // Raw mode + restore on every exit path (including panic).
    let _raw = RawModeGuard::new()?;
    let result = run_session(
        tokio::io::stdin(),
        tokio::io::stdout(),
        &writer,
        notifications,
    )
    .await;

    writer.disconnect().await;
    result.map(|_| ())
}
