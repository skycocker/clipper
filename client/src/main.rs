//! clipper — interactive Flipper Zero CLI shell over Bluetooth.
//!
//! v0 MVP: scan for the Clipper plugin's advertisement, connect, raw-mode
//! pipe between local stdin/stdout and the Flipper's serial GATT
//! characteristics. Exit with Ctrl+] (the telnet escape).

use std::env;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::{Stream, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

/// Flipper Zero serial service (same UUIDs the stock firmware exposes; our
/// plugin reuses them under a different profile identity).
const SERIAL_RX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_62fe_0000);
const SERIAL_TX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_61fe_0000);

/// 16-bit GAP advertising UUID our plugin sets in get_gap_config (0x3081),
/// expanded to 128-bit form via the Bluetooth Base UUID. Matching against
/// this is more reliable than the local_name, which the Flipper sometimes
/// truncates by one byte in the advertisement payload.
const CLIPPER_ADV_UUID: Uuid = Uuid::from_u128(0x0000_3081_0000_1000_8000_0080_5f9b_34fb);

/// Ctrl+]  — telnet-style escape to exit the client cleanly.
const EXIT_KEY: u8 = 0x1d;

/// Default substring to match in the BLE advertisement's local_name.
const DEFAULT_NAME_FILTER: &str = "CLIpper";

const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    let name_filter = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_NAME_FILTER.to_string());

    let manager = Manager::new().await.context("BLE manager init failed")?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .context("no BLE adapter available")?;

    let device = find_device(&adapter, &name_filter, SCAN_TIMEOUT).await?;

    eprintln!("clipper: connecting...");
    tokio::time::timeout(CONNECT_TIMEOUT, device.connect())
        .await
        .context("connect timed out")??;
    device.discover_services().await?;

    let (rx_char, tx_char) = find_serial_chars(&device)?;
    device.subscribe(&tx_char).await?;
    let notifications = device.notifications().await?;

    eprintln!("clipper: connected — type to send, Ctrl+] to exit.\n");

    // From this point on, terminal is in raw mode; restore on every exit path.
    let _raw_guard = RawModeGuard::new()?;
    let result = run_session(&device, &rx_char, notifications).await;

    let _ = device.disconnect().await;
    result
}

async fn find_device(
    adapter: &Adapter,
    name_filter: &str,
    timeout: Duration,
) -> Result<Peripheral> {
    eprintln!("clipper: scanning {timeout:?} for {name_filter:?}...");
    adapter.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(timeout).await;

    let peripherals = adapter.peripherals().await?;
    let debug = env::var("CLIPPER_SCAN_DEBUG").is_ok();
    if debug {
        eprintln!("clipper: scan-debug — {} peripheral(s) seen", peripherals.len());
    }

    let mut best: Option<Peripheral> = None;
    for p in peripherals {
        let props = match p.properties().await? {
            Some(p) => p,
            None => continue,
        };
        let name = props.local_name.clone().unwrap_or_default();
        let name_matches = !name_filter.is_empty()
            && name.to_lowercase().contains(&name_filter.to_lowercase());
        let uuid_matches = props.services.contains(&CLIPPER_ADV_UUID);

        if debug {
            eprintln!(
                "  {}  rssi={:>4?}  name={name:?}  svcs={:?}",
                p.address(),
                props.rssi,
                props.services,
            );
        }

        if !name_matches && !uuid_matches {
            continue;
        }
        eprintln!(
            "clipper: match name={name:?} svc={} rssi={:?}",
            if uuid_matches { "yes" } else { "no" },
            props.rssi
        );
        best = Some(p);
        break;
    }
    let _ = adapter.stop_scan().await;
    best.context(format!(
        "no BLE device matching {name_filter:?} or advertising {CLIPPER_ADV_UUID}. \
         Try CLIPPER_SCAN_DEBUG=1 to see all peripherals btleplug saw."
    ))
}

fn find_serial_chars(device: &Peripheral) -> Result<(Characteristic, Characteristic)> {
    let chars = device.characteristics();
    let rx = chars
        .iter()
        .find(|c| c.uuid == SERIAL_RX)
        .cloned()
        .context("Flipper serial RX characteristic not present")?;
    let tx = chars
        .iter()
        .find(|c| c.uuid == SERIAL_TX)
        .cloned()
        .context("Flipper serial TX characteristic not present")?;
    Ok((rx, tx))
}

async fn run_session(
    device: &Peripheral,
    rx_char: &Characteristic,
    mut notifications: impl Stream<Item = ValueNotification> + Unpin,
) -> Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buf = [0u8; 256];

    loop {
        tokio::select! {
            biased;

            n = stdin.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    return Ok(()); // stdin closed
                }
                if buf[..n].contains(&EXIT_KEY) {
                    return Ok(());
                }
                device
                    .write(rx_char, &buf[..n], WriteType::WithResponse)
                    .await?;
            }

            note = notifications.next() => {
                let Some(note) = note else { bail!("notification stream ended"); };
                stdout.write_all(&note.value).await?;
                stdout.flush().await?;
            }

            _ = tokio::signal::ctrl_c() => {
                // ^C in raw mode usually doesn't fire SIGINT, but if it does
                // we forward 0x03 to the Flipper so the user can interrupt a
                // running command without killing us.
                device
                    .write(rx_char, &[0x03], WriteType::WithResponse)
                    .await?;
            }
        }
    }
}

/// RAII guard that restores cooked-mode on drop, including on panic.
struct RawModeGuard;

impl RawModeGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("enable_raw_mode failed")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}
