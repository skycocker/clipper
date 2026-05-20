//! clipper — interactive Flipper Zero CLI shell over Bluetooth.
//!
//! Thin glue around [`clipper::ble`], [`clipper::session`], and
//! [`clipper::terminal`]. See library docs for the moving parts.

use std::env;
use std::time::Duration;

use anyhow::Result;

use clipper::ble;
use clipper::reconnect::backoff;
use clipper::session::{run_session, SessionExit};
use clipper::terminal::RawModeGuard;

const DEFAULT_NAME_FILTER: &str = "CLIpper";
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

#[tokio::main]
async fn main() -> Result<()> {
    let name_filter = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_NAME_FILTER.to_string());
    let debug = env::var("CLIPPER_SCAN_DEBUG").is_ok();

    // Raw mode + restore on every exit path (including panic).
    let _raw = RawModeGuard::new()?;

    // stdin / stdout are created once and reused across reconnects so we
    // don't fight over the underlying file descriptors.
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let mut attempt: u32 = 0;
    let final_result = loop {
        match ble::connect(&name_filter, SCAN_TIMEOUT, CONNECT_TIMEOUT, debug).await {
            Ok((writer, notifications)) => {
                attempt = 0;
                eprint!("\r\nclipper: connected — type to send, Ctrl+] to exit.\r\n\r\n");
                let outcome = run_session(&mut stdin, &mut stdout, &writer, notifications).await;
                writer.disconnect().await;

                match outcome {
                    Ok(SessionExit::UserExited) | Ok(SessionExit::StdinClosed) => {
                        break Ok(());
                    }
                    Err(e) => {
                        attempt += 1;
                        eprint!(
                            "\r\nclipper: session ended ({e}), reconnecting (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS})...\r\n"
                        );
                        if attempt >= MAX_RECONNECT_ATTEMPTS {
                            break Err(e);
                        }
                        tokio::time::sleep(backoff(attempt)).await;
                    }
                }
            }
            Err(e) => {
                attempt += 1;
                eprint!(
                    "\r\nclipper: connect failed ({e}), retrying (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS})...\r\n"
                );
                if attempt >= MAX_RECONNECT_ATTEMPTS {
                    break Err(e);
                }
                tokio::time::sleep(backoff(attempt)).await;
            }
        }
    };

    final_result
}
