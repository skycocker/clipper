//! BLE abstraction (`FlipperWriter` trait) and btleplug implementation.

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use uuid::Uuid;

/// Stock Flipper-firmware serial-service characteristic UUIDs. Our plugin
/// reuses these so we don't have to redo the GATT layout from scratch.
pub const SERIAL_RX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_62fe_0000);
pub const SERIAL_TX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_61fe_0000);

/// 16-bit GAP advertising UUID our plugin sets (`0x3081`), expanded to its
/// 128-bit Bluetooth-Base-UUID form. Matching this is more reliable than
/// matching `local_name`, which the Flipper truncates by one byte.
pub const CLIPPER_ADV_UUID: Uuid = Uuid::from_u128(0x0000_3081_0000_1000_8000_0080_5f9b_34fb);

/// Anything that can send bytes to the Flipper over a serial-style transport.
/// The session loop is generic over this so tests can substitute a mock.
#[async_trait]
pub trait FlipperWriter: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<()>;
}

/// btleplug-backed writer — writes bytes to the Flipper serial RX
/// characteristic with WithResponse semantics.
pub struct BleWriter {
    peripheral: Peripheral,
    rx_char: Characteristic,
}

#[async_trait]
impl FlipperWriter for BleWriter {
    async fn write(&self, data: &[u8]) -> Result<()> {
        self.peripheral
            .write(&self.rx_char, data, WriteType::WithResponse)
            .await
            .context("BLE write failed")
    }
}

impl BleWriter {
    /// Best-effort cleanup. Errors are swallowed because we're typically on
    /// the way out anyway.
    pub async fn disconnect(self) {
        let _ = self.peripheral.disconnect().await;
    }
}

/// Box<dyn Stream> alias for "bytes arriving from the Flipper", erasing
/// btleplug types so session code doesn't depend on them.
pub type ByteStream = std::pin::Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// Scan, connect, subscribe to TX. Returns a writer for outbound data and
/// a stream of inbound bytes.
pub async fn connect(
    name_filter: &str,
    scan_timeout: Duration,
    connect_timeout: Duration,
    debug: bool,
) -> Result<(BleWriter, ByteStream)> {
    let manager = Manager::new().await.context("BLE manager init failed")?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .context("no BLE adapter available")?;

    let peripheral = find_device(&adapter, name_filter, scan_timeout, debug).await?;

    eprintln!("clipper: connecting...");
    tokio::time::timeout(connect_timeout, peripheral.connect())
        .await
        .context("connect timed out")??;
    peripheral.discover_services().await?;

    let chars = peripheral.characteristics();
    let rx_char = chars
        .iter()
        .find(|c| c.uuid == SERIAL_RX)
        .cloned()
        .context("Flipper serial RX characteristic not present")?;
    let tx_char = chars
        .iter()
        .find(|c| c.uuid == SERIAL_TX)
        .cloned()
        .context("Flipper serial TX characteristic not present")?;

    peripheral.subscribe(&tx_char).await?;
    let raw_stream = peripheral.notifications().await?;
    let byte_stream: ByteStream = Box::pin(raw_stream.map(|n: ValueNotification| n.value));

    Ok((
        BleWriter {
            peripheral,
            rx_char,
        },
        byte_stream,
    ))
}

async fn find_device(
    adapter: &Adapter,
    name_filter: &str,
    timeout: Duration,
    debug: bool,
) -> Result<Peripheral> {
    eprintln!("clipper: scanning {timeout:?} for {name_filter:?}...");
    adapter.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(timeout).await;

    let peripherals = adapter.peripherals().await?;
    if debug {
        eprintln!(
            "clipper: scan-debug — {} peripheral(s) seen",
            peripherals.len()
        );
    }

    let mut best: Option<Peripheral> = None;
    for p in peripherals {
        let Some(props) = p.properties().await? else {
            continue;
        };

        let name = props.local_name.clone().unwrap_or_default();
        let name_matches =
            !name_filter.is_empty() && name.to_lowercase().contains(&name_filter.to_lowercase());
        let uuid_matches = props.services.contains(&CLIPPER_ADV_UUID);

        if debug {
            eprintln!(
                "  {}  rssi={:>4?}  name={name:?}  svcs={:?}",
                p.address(),
                props.rssi,
                props.services,
            );
        }

        if name_matches || uuid_matches {
            eprintln!(
                "clipper: match name={name:?} svc={} rssi={:?}",
                if uuid_matches { "yes" } else { "no" },
                props.rssi
            );
            best = Some(p);
            break;
        }
    }
    let _ = adapter.stop_scan().await;
    best.context(format!(
        "no BLE device matching {name_filter:?} or advertising {CLIPPER_ADV_UUID}. \
         Try CLIPPER_SCAN_DEBUG=1 to dump every peripheral btleplug saw."
    ))
}
