//! The bidirectional pipe between local stdin/stdout and the Flipper. Pure
//! glue, no platform/IO assumptions beyond the trait bounds — so tests can
//! drive it with mock streams.

use std::time::Duration;

use anyhow::{bail, Result};
use futures::{Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::ble::FlipperWriter;

/// How often to poll `FlipperWriter::is_connected` to catch silent disconnects.
const LIVENESS_INTERVAL: Duration = Duration::from_millis(500);

/// Byte sequences in stdin that exit the session loop cleanly. Any byte from
/// this set, when seen in a stdin chunk, ends the session. We accept multiple
/// because terminal emulators and shells sometimes intercept individual ones:
///   - 0x1d  Ctrl+]   (telnet-style, the "preferred" key)
///   - 0x1c  Ctrl+\\   (file separator; rarely bound by shells)
///   - 0x04  Ctrl+D   (EOT; in raw mode shells don't translate it to EOF)
pub const EXIT_KEYS: &[u8] = &[0x1d, 0x1c, 0x04];

/// First key from EXIT_KEYS — used in tests and as the canonical "what to
/// send in test inputs".
pub const EXIT_KEY: u8 = EXIT_KEYS[0];

/// Byte forwarded to the Flipper when the host process receives SIGINT. The
/// Flipper CLI interprets 0x03 as an interrupt so the user can break a
/// running command without killing the clipper client.
const REMOTE_INTERRUPT: u8 = 0x03;

const STDIN_CHUNK: usize = 256;

/// Outcome of a session run — encoded to make tests assertable rather than
/// returning bare `()`. Errors are still bubbled via `Result`.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionExit {
    /// User typed the EXIT_KEY (Ctrl+]).
    UserExited,
    /// Stdin reached EOF (e.g. test driver dropped its sender).
    StdinClosed,
}

/// Pump bytes between `stdin`/`stdout` and the Flipper via `writer` /
/// `notifications`. Blocks until the user exits, stdin closes, or an error
/// surfaces from any side.
pub async fn run_session<R, W, S>(
    stdin: &mut R,
    stdout: &mut W,
    writer: &dyn FlipperWriter,
    mut notifications: S,
) -> Result<SessionExit>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: Stream<Item = Vec<u8>> + Unpin,
{
    let mut buf = [0u8; STDIN_CHUNK];
    let mut liveness = tokio::time::interval(LIVENESS_INTERVAL);
    // Skip the immediate first tick — interval fires once at t=0 by default.
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    liveness.tick().await;

    loop {
        tokio::select! {
            biased;

            n = stdin.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    return Ok(SessionExit::StdinClosed);
                }
                if let Some(idx) = buf[..n].iter().position(|b| EXIT_KEYS.contains(b)) {
                    // Forward everything *before* the exit byte so any final
                    // CR isn't dropped, then exit.
                    if idx > 0 {
                        writer.write(&buf[..idx]).await?;
                    }
                    return Ok(SessionExit::UserExited);
                }
                writer.write(&buf[..n]).await?;
            }

            note = notifications.next() => {
                let Some(bytes) = note else { bail!("notification stream ended") };
                stdout.write_all(&bytes).await?;
                stdout.flush().await?;
            }

            _ = tokio::signal::ctrl_c() => {
                writer.write(&[REMOTE_INTERRUPT]).await?;
            }

            _ = liveness.tick() => {
                if !writer.is_connected().await {
                    bail!("peripheral disconnected");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    /// Records every byte written via `FlipperWriter` for later assertion.
    struct MockWriter(Mutex<Vec<u8>>);

    impl MockWriter {
        fn new() -> Self {
            Self(Mutex::new(Vec::new()))
        }
        fn taken(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FlipperWriter for MockWriter {
        async fn write(&self, data: &[u8]) -> Result<()> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(())
        }
        async fn is_connected(&self) -> bool {
            true
        }
    }

    /// A FlipperWriter that flips to disconnected after N polls of
    /// is_connected. Used to exercise the liveness-check path.
    struct DisconnectingWriter {
        polls_until_dead: std::sync::atomic::AtomicU32,
    }

    impl DisconnectingWriter {
        fn new(polls_until_dead: u32) -> Self {
            Self {
                polls_until_dead: std::sync::atomic::AtomicU32::new(polls_until_dead),
            }
        }
    }

    #[async_trait]
    impl FlipperWriter for DisconnectingWriter {
        async fn write(&self, _: &[u8]) -> Result<()> {
            Ok(())
        }
        async fn is_connected(&self) -> bool {
            use std::sync::atomic::Ordering::Relaxed;
            let remaining = self.polls_until_dead.fetch_sub(1, Relaxed);
            remaining > 0
        }
    }

    /// Drive a session end-to-end: provide a vec of stdin chunks and a vec of
    /// notification chunks, return (exit, bytes_written_to_ble, bytes_written_to_stdout).
    async fn drive(
        stdin_chunks: Vec<Vec<u8>>,
        notif_chunks: Vec<Vec<u8>>,
    ) -> (SessionExit, Vec<u8>, Vec<u8>) {
        let (mut stdin_writer, stdin_reader) = tokio::io::duplex(1024);
        let (mut stdout_reader, stdout_writer) = tokio::io::duplex(1024);
        let (notif_tx, notif_rx) = mpsc::channel(16);
        let writer = std::sync::Arc::new(MockWriter::new());
        let writer_clone = writer.clone();

        let session = tokio::spawn(async move {
            let mut stdin_reader = stdin_reader;
            let mut stdout_writer = stdout_writer;
            let notifications = ReceiverStream::new(notif_rx);
            run_session(
                &mut stdin_reader,
                &mut stdout_writer,
                writer_clone.as_ref(),
                notifications,
            )
            .await
        });

        // Feed all notifications first and give the session loop time to
        // drain them BEFORE any stdin arrives. The session loop has a biased
        // select that prefers stdin, so if we mix in stdin chunks
        // immediately we may exit before notifications get serviced.
        for n in notif_chunks {
            notif_tx.send(n).await.unwrap();
        }
        if !notif_tx.is_closed() {
            // Yield repeatedly until the channel is drained by the session task.
            for _ in 0..50 {
                if notif_tx.capacity() == notif_tx.max_capacity() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }

        // Now feed stdin chunks with small delays so each one is processed
        // before the next arrives.
        for chunk in stdin_chunks {
            stdin_writer.write_all(&chunk).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        drop(stdin_writer); // signal EOF if no EXIT_KEY was sent
        drop(notif_tx);

        let exit = tokio::time::timeout(std::time::Duration::from_secs(2), session)
            .await
            .expect("session timed out")
            .unwrap()
            .unwrap();

        // Drain stdout
        let mut stdout_bytes = Vec::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            tokio::io::AsyncReadExt::read_to_end(&mut stdout_reader, &mut stdout_bytes),
        )
        .await;

        (exit, writer.taken(), stdout_bytes)
    }

    #[tokio::test]
    async fn stdin_bytes_forwarded_to_ble_writer() {
        let (exit, ble_writes, _stdout) =
            drive(vec![b"hello\r".to_vec(), vec![EXIT_KEY]], vec![]).await;
        assert_eq!(exit, SessionExit::UserExited);
        assert_eq!(&ble_writes, b"hello\r");
    }

    #[tokio::test]
    async fn ble_notifications_forwarded_to_stdout() {
        let (exit, _ble, stdout) = drive(
            vec![vec![EXIT_KEY]],
            vec![b">: ".to_vec(), b"ready\n".to_vec()],
        )
        .await;
        assert_eq!(exit, SessionExit::UserExited);
        assert!(
            stdout.windows(3).any(|w| w == b">: "),
            "missing prompt: {stdout:?}"
        );
        assert!(stdout.windows(5).any(|w| w == b"ready"));
    }

    #[tokio::test]
    async fn exit_key_alone_exits_cleanly() {
        let (exit, ble, _stdout) = drive(vec![vec![EXIT_KEY]], vec![]).await;
        assert_eq!(exit, SessionExit::UserExited);
        assert!(
            ble.is_empty(),
            "no bytes should be forwarded if input is only the exit key"
        );
    }

    #[tokio::test]
    async fn exit_key_after_partial_input_flushes_pre_bytes() {
        let mut input = b"ab".to_vec();
        input.push(EXIT_KEY);
        let (exit, ble, _stdout) = drive(vec![input], vec![]).await;
        assert_eq!(exit, SessionExit::UserExited);
        assert_eq!(&ble, b"ab");
    }

    #[tokio::test]
    async fn liveness_check_returns_err_when_peer_disconnects() {
        // Liveness ticks every 500ms; let the mock disconnect on the first
        // post-tick poll.
        let writer = DisconnectingWriter::new(0);
        let (stdin_writer, stdin_reader) = tokio::io::duplex(64);
        let (_, stdout_writer) = tokio::io::duplex(64);
        let (_notif_tx, notif_rx) = mpsc::channel::<Vec<u8>>(1);

        let session = tokio::spawn(async move {
            let mut stdin_reader = stdin_reader;
            let mut stdout_writer = stdout_writer;
            let notifications = ReceiverStream::new(notif_rx);
            run_session(
                &mut stdin_reader,
                &mut stdout_writer,
                &writer,
                notifications,
            )
            .await
        });

        // Keep stdin open so EOF doesn't race the liveness check.
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), session)
            .await
            .expect("session should error within 3s of liveness tick")
            .unwrap();

        assert!(result.is_err(), "expected disconnect error, got {result:?}");
        drop(stdin_writer);
    }

    #[tokio::test]
    async fn stdin_eof_exits_cleanly() {
        // No EXIT_KEY, no stdin chunks beyond the initial — drop signals EOF.
        let (exit, ble, _stdout) = drive(vec![], vec![]).await;
        assert_eq!(exit, SessionExit::StdinClosed);
        assert!(ble.is_empty());
    }
}
