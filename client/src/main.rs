//! clipper — interactive Flipper Zero CLI shell over Bluetooth.
//!
//! Two modes:
//!   clipper [NAME]              interactive: bridge your terminal to the Flipper
//!   clipper --listen ADDR [NAME]  serve: bridge a TCP socket to the Flipper, so
//!                                 a remote machine (or automation, e.g. an agent)
//!                                 can drive the CLI over the network
//!
//! Thin glue around [`clipper::ble`], [`clipper::session`], and
//! [`clipper::terminal`]. See library docs for the moving parts.

use std::env;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use clipper::ble::{self, BleWriter};
use clipper::reconnect::backoff;
use clipper::session::run_session;
use clipper::terminal::RawModeGuard;

const DEFAULT_NAME_FILTER: &str = "CLIpper";
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

struct Args {
    name: String,
    listen: Option<String>,
    debug: bool,
}

fn parse_args() -> Result<Args> {
    let mut name = DEFAULT_NAME_FILTER.to_string();
    let mut listen = None;
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-l" | "--listen" => {
                let v = argv
                    .get(i + 1)
                    .context("--listen needs an address, e.g. --listen 127.0.0.1:2323")?;
                listen = Some(normalize_listen_addr(v));
                i += 2;
            }
            other => {
                name = other.to_string();
                i += 1;
            }
        }
    }
    Ok(Args {
        name,
        listen,
        debug: env::var("CLIPPER_SCAN_DEBUG").is_ok(),
    })
}

/// Accept a bare port ("2323"), an addr ("127.0.0.1:2323"), or "host:port".
/// A bare port binds loopback only — never expose the Flipper CLI to the whole
/// network by accident.
fn normalize_listen_addr(v: &str) -> String {
    if v.contains(':') {
        v.to_string()
    } else {
        format!("127.0.0.1:{v}")
    }
}

fn print_help() {
    eprintln!(
        "clipper — the Flipper Zero CLI over Bluetooth\n\n\
         USAGE:\n\
         \x20 clipper [NAME]                 interactive shell in this terminal\n\
         \x20 clipper --listen ADDR [NAME]   serve the shell on a TCP socket\n\n\
         ARGS:\n\
         \x20 NAME   advertised-name substring to match (default: \"CLIpper\")\n\n\
         OPTIONS:\n\
         \x20 -l, --listen ADDR   bind a TCP listener (ADDR = PORT | IP:PORT).\n\
         \x20                     A bare PORT binds 127.0.0.1 only.\n\
         \x20 -h, --help          show this help\n\n\
         ENV:\n\
         \x20 CLIPPER_SCAN_DEBUG=1   dump every BLE peripheral seen while scanning\n\n\
         SERVE MODE connects to the Flipper once and bridges each TCP client to it.\n\
         Anyone who can reach the port controls the Flipper — bind loopback and use\n\
         an SSH tunnel for remote access (ssh -L 2323:127.0.0.1:2323 host)."
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    if args.listen.is_some() {
        serve(&args).await
    } else {
        interactive(&args).await
    }
}

/// Interactive: bridge local stdin/stdout to the Flipper, with reconnect.
async fn interactive(args: &Args) -> Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let mut attempt: u32 = 0;
    loop {
        // Stay in COOKED mode during scan/connect (status lines end in \n);
        // raw mode is enabled only around the session.
        match ble::connect(&args.name, SCAN_TIMEOUT, CONNECT_TIMEOUT, args.debug).await {
            Ok(writer) => {
                attempt = 0;
                eprintln!("\nclipper: connected — type to send, Ctrl+] (or Ctrl+\\, Ctrl+D) to exit.\n");
                let outcome = async {
                    let notifications = writer.notifications().await?;
                    let _raw = RawModeGuard::new()?;
                    run_session(&mut stdin, &mut stdout, &writer, notifications, true).await
                }
                .await;
                writer.disconnect().await;

                match outcome {
                    Ok(_) => return Ok(()),
                    Err(e) => {
                        attempt += 1;
                        eprint!("\r\nclipper: session ended ({e}), reconnecting (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS})...\r\n");
                        if attempt >= MAX_RECONNECT_ATTEMPTS {
                            return Err(e);
                        }
                        tokio::time::sleep(backoff(attempt)).await;
                    }
                }
            }
            Err(e) => {
                attempt += 1;
                eprintln!("clipper: connect failed ({e}), retrying (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS})...");
                if attempt >= MAX_RECONNECT_ATTEMPTS {
                    return Err(e);
                }
                tokio::time::sleep(backoff(attempt)).await;
            }
        }
    }
}

/// Serve: bridge each accepted TCP client to the Flipper. One client at a time
/// (the Flipper has a single CLI session). BLE is connected fresh per client —
/// right before the session, never held idle — which mirrors the proven
/// interactive flow and avoids a stale link going dead between clients.
async fn serve(args: &Args) -> Result<()> {
    let addr = args.listen.as_deref().unwrap();
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    let local = listener.local_addr()?;
    eprintln!("clipper: listening on {local} (waiting for clients)");
    if !local.ip().is_loopback() {
        eprintln!(
            "clipper: WARNING — {local} is not loopback. Anyone who can reach this\n\
             port gets full control of the Flipper CLI. Prefer binding 127.0.0.1 and\n\
             tunnelling over SSH (ssh -L {p}:127.0.0.1:{p} <host>).",
            p = local.port()
        );
    }

    loop {
        let (sock, peer) = listener.accept().await.context("accept failed")?;
        sock.set_nodelay(true).ok();
        eprintln!("clipper: client {peer} connected; connecting to Flipper...");

        let writer: BleWriter =
            match ble::connect(&args.name, SCAN_TIMEOUT, CONNECT_TIMEOUT, args.debug).await {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("clipper: BLE connect failed for {peer}: {e}");
                    continue; // drop this client, wait for the next
                }
            };

        let session = async {
            let notifications = writer.notifications().await?;
            let (mut rd, mut wr) = sock.into_split();
            // handle_sigint = false: a server must not swallow its own Ctrl+C;
            // remote clients send 0x03 as a raw byte through the socket.
            run_session(&mut rd, &mut wr, &writer, notifications, false).await
        }
        .await;

        writer.disconnect().await;
        match session {
            Ok(exit) => eprintln!("clipper: client {peer} disconnected ({exit:?})"),
            Err(e) => eprintln!("clipper: client {peer} session error ({e})"),
        }
    }
}
