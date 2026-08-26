//! In-memory [`SerialIo`] implementations for unit and integration tests.
//! Integration tests can build a [`SerialConnection`] backed by
//! [`tokio::io::DuplexStream`] without using the OS serial layer.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

use super::*;

/// Wraps one [`DuplexStream`] half as a [`SerialIo`] backend.
/// All control-line operations are no-ops.
pub struct LoopbackIo(DuplexStream);

impl AsyncRead for LoopbackIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for LoopbackIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl SerialIo for LoopbackIo {
    fn clear_os_buffers(&self, _target: FlushTarget) -> std::io::Result<()> {
        Ok(())
    }
    fn set_dtr_rts(&mut self, _dtr: bool, _rts: bool) -> std::io::Result<()> {
        Ok(())
    }
    fn set_flow_control(&mut self, _flow_control: FlowControl) -> std::io::Result<()> {
        Ok(())
    }
    fn set_break_state(&self, _asserted: bool) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build an in-memory connection plus the peer end of the duplex.
/// The peer can be driven directly by the test to push bytes into the
/// connection's read side or to consume bytes the connection writes.
pub fn loopback_connection(port: &str) -> (SerialConnection, DuplexStream) {
    let (a, b) = tokio::io::duplex(4096);
    let conn = SerialConnection::from_io(port.to_string(), Box::new(LoopbackIo(a)));
    (conn, b)
}

pub fn loopback_connection_with_config(
    config: ConnectionConfig,
) -> (SerialConnection, DuplexStream) {
    let (a, b) = tokio::io::duplex(4096);
    let conn = SerialConnection::from_io_with_config(config, Box::new(LoopbackIo(a)));
    (conn, b)
}

// QueuedTxIo.
//
// This `SerialIo` backend models an OS transmit queue: `write()` enqueues
// bytes, output flush discards queued bytes before device drain, and input
// flush discards injected RX bytes. Unlike `LoopbackIo`, it keeps written bytes
// queued until the test drains them through `QueuedTxHandle`. The RX direction
// is fed by `QueuedTxHandle::inject_rx`, which wakes pending reads.

/// State shared by [`QueuedTxIo`] and [`QueuedTxHandle`].
struct QueuedTxState {
    /// Bytes written by the host, awaiting delivery to the device.
    /// `clear_os_buffers(Output)` discards these.
    tx_queue: std::collections::VecDeque<u8>,
    /// Bytes the device injected for the host to read.
    /// `clear_os_buffers(Input)` discards these.
    rx_queue: std::collections::VecDeque<u8>,
    /// Waker for the host-side `poll_read`, notified when rx_queue grows.
    rx_waker: Option<std::task::Waker>,
}

impl QueuedTxState {
    fn new() -> Self {
        Self {
            tx_queue: std::collections::VecDeque::new(),
            rx_queue: std::collections::VecDeque::new(),
            rx_waker: None,
        }
    }
}

/// A `SerialIo` backend whose TX path models an OS transmit queue.
pub struct QueuedTxIo {
    state: std::sync::Arc<std::sync::Mutex<QueuedTxState>>,
}

/// Handle for simulating device reads, device writes, and queue inspection.
pub struct QueuedTxHandle {
    state: std::sync::Arc<std::sync::Mutex<QueuedTxState>>,
}

/// Build a [`SerialConnection`] backed by [`QueuedTxIo`] and a handle for the
/// simulated device side.
pub fn queued_tx_connection(port: &str) -> (SerialConnection, QueuedTxHandle) {
    let state = std::sync::Arc::new(std::sync::Mutex::new(QueuedTxState::new()));
    let conn = SerialConnection::from_io(
        port.to_string(),
        Box::new(QueuedTxIo {
            state: state.clone(),
        }),
    );
    (conn, QueuedTxHandle { state })
}

impl QueuedTxHandle {
    /// Number of host-written bytes still queued for device delivery.
    pub fn tx_queue_len(&self) -> usize {
        self.state.lock().expect("tx state poisoned").tx_queue.len()
    }

    /// Drain up to `max` queued bytes, simulating the device reading its RX.
    /// Returns drained bytes in order.
    pub fn drain_tx(&self, max: usize) -> Vec<u8> {
        let mut state = self.state.lock().expect("tx state poisoned");
        let take = max.min(state.tx_queue.len());
        let mut out = Vec::with_capacity(take);
        for _ in 0..take {
            out.push(state.tx_queue.pop_front().expect("len checked"));
        }
        out
    }

    /// Inject bytes for the host to read, simulating a device write.
    pub fn inject_rx(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("tx state poisoned");
        state.rx_queue.extend(bytes);
        if let Some(w) = state.rx_waker.take() {
            w.wake();
        }
    }

    /// Number of bytes currently buffered for the host to read.
    pub fn rx_queue_len(&self) -> usize {
        self.state.lock().expect("tx state poisoned").rx_queue.len()
    }
}

impl AsyncRead for QueuedTxIo {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut state = self.state.lock().expect("tx state poisoned");
        let n = buf.initialize_unfilled().len().min(state.rx_queue.len());
        if n == 0 {
            // Remember the waker so a later inject_rx can wake us.
            state.rx_waker = Some(_cx.waker().clone());
            return Poll::Pending;
        }
        let filled = buf.initialize_unfilled();
        for slot in filled.iter_mut().take(n) {
            *slot = state.rx_queue.pop_front().expect("len checked");
        }
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for QueuedTxIo {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut state = self.state.lock().expect("tx state poisoned");
        state.tx_queue.extend(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Bytes stay in tx_queue until drain_tx or clear_os_buffers(Output).
        // A successful write does not imply that the device consumed them.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl SerialIo for QueuedTxIo {
    fn clear_os_buffers(&self, target: FlushTarget) -> std::io::Result<()> {
        let mut state = self.state.lock().expect("tx state poisoned");
        match target {
            FlushTarget::Input => {
                state.rx_queue.clear();
            }
            FlushTarget::Output => {
                state.tx_queue.clear();
            }
            FlushTarget::Both => {
                state.tx_queue.clear();
                state.rx_queue.clear();
            }
        }
        Ok(())
    }
    fn set_dtr_rts(&mut self, _dtr: bool, _rts: bool) -> std::io::Result<()> {
        Ok(())
    }
    fn set_flow_control(&mut self, _flow_control: FlowControl) -> std::io::Result<()> {
        Ok(())
    }
    fn set_break_state(&self, _asserted: bool) -> std::io::Result<()> {
        Ok(())
    }
}
