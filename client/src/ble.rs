//! BLE abstraction (`FlipperWriter` trait) and btleplug implementation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use uuid::Uuid;

/// Stock Flipper-firmware serial-service characteristic UUIDs. Our plugin
/// reuses these so we don't have to redo the GATT layout from scratch.
pub const SERIAL_RX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_62fe_0000);
pub const SERIAL_TX: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_61fe_0000);
/// Flow-control char. Read-only from host's POV, side-effect-free, fixed
/// 4 bytes — perfect cheap liveness probe.
pub const SERIAL_FLOW_CTRL: Uuid = Uuid::from_u128(0x19ed_82ae_ed21_4c9d_4145_228e_63fe_0000);

/// 16-bit GAP advertising UUID our plugin sets (`0x3081`), expanded to its
/// 128-bit Bluetooth-Base-UUID form. Matching this is more reliable than
/// matching `local_name`, which the Flipper truncates by one byte.
pub const CLIPPER_ADV_UUID: Uuid = Uuid::from_u128(0x0000_3081_0000_1000_8000_0080_5f9b_34fb);

/// Anything that can send bytes to the Flipper over a serial-style transport.
/// The session loop is generic over this so tests can substitute a mock.
#[async_trait]
pub trait FlipperWriter: Send + Sync {
    /// Send bytes toward the Flipper.
    async fn write(&self, data: &[u8]) -> Result<()>;

    /// Lightweight liveness check. Polled periodically by the session loop
    /// so we can detect disconnects that the notification stream doesn't
    /// surface (btleplug on macOS keeps the stream open after disconnect
    /// rather than ending it).
    async fn is_connected(&self) -> bool;
}

/// btleplug-backed writer — writes bytes to the Flipper serial RX
/// characteristic with WithResponse semantics. Also owns a watcher task that
/// subscribes to adapter events and flips `disconnected` when the peripheral
/// drops, since `Peripheral::is_connected()` doesn't update reactively on
/// macOS without an active subscription.
pub struct BleWriter {
    peripheral: Peripheral,
    rx_char: Characteristic,
    probe_char: Characteristic,
    disconnected: Arc<AtomicBool>,
    watcher: Option<tokio::task::JoinHandle<()>>,
}

#[async_trait]
impl FlipperWriter for BleWriter {
    async fn write(&self, data: &[u8]) -> Result<()> {
        self.peripheral
            .write(&self.rx_char, data, WriteType::WithResponse)
            .await
            .context("BLE write failed")
    }

    async fn is_connected(&self) -> bool {
        // Two signals: the event-watcher's atomic flag and a real GATT read.
        // Known macOS limitation: btleplug's adapter events and the
        // underlying CoreBluetooth GATT read can take many seconds to
        // notice a silent peer (peer reset its BLE chip rather than sending
        // LL_TERMINATE_IND). Our reconnect loop is correct regardless; it
        // just won't kick in immediately on macOS. Linux/BlueZ surfaces
        // disconnects much faster.
        if self.disconnected.load(Ordering::Relaxed) {
            return false;
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(800),
            self.peripheral.read(&self.probe_char),
        )
        .await;
        matches!(result, Ok(Ok(_)))
    }
}

impl BleWriter {
    /// Best-effort cleanup. Errors are swallowed because we're typically on
    /// the way out anyway.
    pub async fn disconnect(mut self) {
        if let Some(h) = self.watcher.take() {
            h.abort();
        }
        let _ = self.peripheral.disconnect().await;
    }

    /// A fresh stream of bytes arriving from the Flipper (TX indications).
    /// The TX characteristic is already subscribed by `connect()`, so this
    /// can be called repeatedly — e.g. once per network client in serve mode
    /// — to bridge the same persistent BLE connection to a new session.
    pub async fn notifications(&self) -> Result<ByteStream> {
        let raw = self.peripheral.notifications().await?;
        Ok(Box::pin(raw.map(|n: ValueNotification| n.value)))
    }
}

impl Drop for BleWriter {
    fn drop(&mut self) {
        if let Some(h) = self.watcher.take() {
            h.abort();
        }
    }
}

/// Box<dyn Stream> alias for "bytes arriving from the Flipper", erasing
/// btleplug types so session code doesn't depend on them.
pub type ByteStream = std::pin::Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// Scan, connect, subscribe to TX. Returns a writer for outbound data; get an
/// inbound byte stream from [`BleWriter::notifications`] (call it per session).
pub async fn connect(
    name_filter: &str,
    scan_timeout: Duration,
    connect_timeout: Duration,
    debug: bool,
) -> Result<BleWriter> {
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
    let probe_char = chars
        .iter()
        .find(|c| c.uuid == SERIAL_FLOW_CTRL)
        .cloned()
        .context("Flipper flow-control characteristic not present")?;

    peripheral.subscribe(&tx_char).await?;

    // Watch adapter events for DeviceDisconnected on our peripheral.
    let disconnected = Arc::new(AtomicBool::new(false));
    let watcher = spawn_disconnect_watcher(adapter, peripheral.id(), disconnected.clone()).await?;

    Ok(BleWriter {
        peripheral,
        rx_char,
        probe_char,
        disconnected,
        watcher: Some(watcher),
    })
}

async fn spawn_disconnect_watcher(
    adapter: Adapter,
    target_id: btleplug::platform::PeripheralId,
    disconnected: Arc<AtomicBool>,
) -> Result<tokio::task::JoinHandle<()>> {
    let mut events = adapter.events().await?;
    Ok(tokio::spawn(async move {
        while let Some(event) = events.next().await {
            if let CentralEvent::DeviceDisconnected(id) = event {
                if id == target_id {
                    disconnected.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }))
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
