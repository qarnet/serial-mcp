//! Controlled `SerialIo` backend for `capture_boot` tests.
//!
//! A real in-memory byte stream (like `QueuedTxIo`) plus observability hooks
//! that let tests prove reset-pulse atomicity through the PUBLIC MCP surface
//! without mocking tool or store logic:
//!
//! - `line_log` records every `set_dtr_rts` call in order — tests assert
//!   assert/release ordering and that no line-control call can interleave
//!   inside a pulse.
//! - `on_line_change` fires SYNCHRONOUSLY inside the line-change call, so a
//!   test can inject RX bytes at the exact instant the reset line is
//!   asserted/released (the "immediate boot bytes" case).
//! - `fail_next_set` makes the next `set_dtr_rts` fail (assert or release
//!   failure injection).
//! - `os_input_flush_count` counts `clear_os_buffers(Input)` calls so tests
//!   can assert the purge happened before any line assertion.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use serial_mcp::serial::{
    ConnectionConfig, DataBits, FlowControl, FlushTarget, Parity, SerialConnection, SerialIo,
    StopBits,
};

/// Shared observable state between the backend and the test.
pub struct ControlledState {
    rx_queue: StdMutex<VecDeque<u8>>,
    rx_waker: StdMutex<Option<Waker>>,
    /// Every `set_dtr_rts` call, in order: `(dtr, rts)`.
    line_log: StdMutex<Vec<(bool, bool)>>,
    /// Number of `set_dtr_rts` calls to fail (assert/release injection).
    fail_next_set: AtomicUsize,
    /// Synchronous hook fired inside `set_dtr_rts` (before recording). The
    /// hook receives the line state being set so tests can inject RX bytes
    /// only at the assertion instant (not at release).
    #[allow(clippy::type_complexity)]
    on_line_change: StdMutex<Option<Arc<dyn Fn(bool, bool) + Send + Sync>>>,
    /// Count of `clear_os_buffers(Input)` calls.
    os_input_flush_count: AtomicUsize,
}

impl ControlledState {
    fn new() -> Self {
        Self {
            rx_queue: StdMutex::new(VecDeque::new()),
            rx_waker: StdMutex::new(None),
            line_log: StdMutex::new(Vec::new()),
            fail_next_set: AtomicUsize::new(0),
            on_line_change: StdMutex::new(None),
            os_input_flush_count: AtomicUsize::new(0),
        }
    }

    /// Inject bytes the host can read (simulates the device writing).
    pub fn inject_rx(&self, bytes: &[u8]) {
        {
            let mut queue = self.rx_queue.lock().expect("rx queue poisoned");
            queue.extend(bytes);
        }
        self.wake_reader();
    }

    /// Wake a reader parked in `poll_read` (after injecting bytes).
    pub fn wake_reader(&self) {
        if let Some(w) = self.rx_waker.lock().expect("waker poisoned").take() {
            w.wake();
        }
    }

    /// All `set_dtr_rts` calls so far, in order.
    pub fn line_log(&self) -> Vec<(bool, bool)> {
        self.line_log.lock().expect("line log poisoned").clone()
    }

    /// Fail the next `n` `set_dtr_rts` calls with an I/O error.
    pub fn set_fail_next_set(&self, n: usize) {
        self.fail_next_set.store(n, Ordering::SeqCst);
    }

    /// Install (or clear) a hook fired synchronously inside every
    /// `set_dtr_rts` call — used to inject RX bytes at assertion/release
    /// time. The hook receives the line state `(dtr, rts)` being set.
    pub fn set_on_line_change(&self, hook: Option<Arc<dyn Fn(bool, bool) + Send + Sync>>) {
        *self.on_line_change.lock().expect("hook poisoned") = hook;
    }

    /// Count of `clear_os_buffers(Input)` calls.
    pub fn os_input_flush_count(&self) -> usize {
        self.os_input_flush_count.load(Ordering::SeqCst)
    }

    /// Bytes currently queued for the host to read (unpurged OS input).
    pub fn rx_queue_len(&self) -> usize {
        self.rx_queue.lock().expect("rx queue poisoned").len()
    }
}

/// The backend itself. Owns nothing; all state lives in `ControlledState`.
pub struct ControlledIo {
    state: Arc<ControlledState>,
}

/// Build a `SerialConnection` backed by a [`ControlledIo`] with a specific
/// ring size (the connection stores `rx_buffer_size`; `capture_boot` creates
/// the RX session with it).
pub fn controlled_connection(
    port: &str,
    rx_buffer_size: usize,
) -> (SerialConnection, Arc<ControlledState>) {
    let state = Arc::new(ControlledState::new());
    let config = ConnectionConfig {
        port: port.into(),
        name: None,
        baud_rate: 115200,
        data_bits: DataBits::Eight,
        stop_bits: StopBits::One,
        parity: Parity::None,
        flow_control: FlowControl::None,
        port_info: None,
        log_capacity: 1024,
        log_enabled: true,
        tx_framing: None,
        rx_framing: None,
        rx_parser: None,
        protocol: None,
        rx_buffer_size,
        max_buffered_bytes: 32768,
    };
    let conn = SerialConnection::from_io_with_config(
        config,
        Box::new(ControlledIo {
            state: Arc::clone(&state),
        }),
    );
    (conn, state)
}

/// Cross-platform [`serial_mcp::serial::ConnectionOpener`]: builds every
/// connection from the EXACT config passed by the public `open` path,
/// backed by a fresh [`ControlledIo`]. This lets the full MCP surface
/// (allowlist, identity capture, profile session, resource hints) run
/// without an OS serial port — which also keeps it working on macOS and
/// Windows where tty/PTY opens are unavailable or fail (macOS tty open
/// returns ENOTTY).
pub struct ControlledConnectionOpener {
    /// Every connection's shared state, in open order.
    states: StdMutex<Vec<Arc<ControlledState>>>,
}

impl ControlledConnectionOpener {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            states: StdMutex::new(Vec::new()),
        })
    }

    /// All [`ControlledState`]s created so far, in open order.
    pub fn states(&self) -> Vec<Arc<ControlledState>> {
        self.states.lock().expect("states poisoned").clone()
    }
}

#[async_trait::async_trait]
impl serial_mcp::serial::ConnectionOpener for ControlledConnectionOpener {
    async fn open(&self, config: ConnectionConfig) -> serial_mcp::error::Result<SerialConnection> {
        let state = Arc::new(ControlledState::new());
        self.states
            .lock()
            .expect("states poisoned")
            .push(Arc::clone(&state));
        let conn = SerialConnection::from_io_with_config(config, Box::new(ControlledIo { state }));
        Ok(conn)
    }
}

impl AsyncRead for ControlledIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut queue = self.state.rx_queue.lock().expect("rx queue poisoned");
        if queue.is_empty() {
            // Park with the waker registered so a later `inject_rx` wakes us.
            *self.state.rx_waker.lock().expect("waker poisoned") = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = buf.initialize_unfilled().len().min(queue.len());
        let filled = buf.initialize_unfilled();
        for slot in filled.iter_mut().take(n) {
            *slot = queue.pop_front().expect("len checked");
        }
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for ControlledIo {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl SerialIo for ControlledIo {
    fn clear_os_buffers(&self, target: FlushTarget) -> std::io::Result<()> {
        if matches!(target, FlushTarget::Input | FlushTarget::Both) {
            self.state
                .rx_queue
                .lock()
                .expect("rx queue poisoned")
                .clear();
            self.state
                .os_input_flush_count
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn set_dtr_rts(&mut self, dtr: bool, rts: bool) -> std::io::Result<()> {
        // Failure injection: record the attempted state (DTR applied, RTS
        // failed — like the production setter's partial failure), then fail.
        if self.state.fail_next_set.swap(0, Ordering::SeqCst) > 0 {
            self.state
                .line_log
                .lock()
                .expect("line log poisoned")
                .push((dtr, rts));
            return Err(std::io::Error::other("injected set_dtr_rts failure"));
        }
        // Fire the hook synchronously INSIDE the line-change call so tests
        // can emit bytes at the exact assertion/release instant.
        if let Some(hook) = self
            .state
            .on_line_change
            .lock()
            .expect("hook poisoned")
            .as_ref()
        {
            hook(dtr, rts);
        }
        self.state
            .line_log
            .lock()
            .expect("line log poisoned")
            .push((dtr, rts));
        Ok(())
    }

    fn set_flow_control(&mut self, _flow_control: FlowControl) -> std::io::Result<()> {
        Ok(())
    }

    fn set_break_state(&self, _asserted: bool) -> std::io::Result<()> {
        Ok(())
    }
}
