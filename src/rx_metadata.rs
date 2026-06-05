//! Shared RX stop metadata for all serial data tools.
//!
//! Every tool that consumes from the RX session (`read`, `wait_for`,
//! `subscribe`) produces the same stop metadata vocabulary so clients can
//! reason about why an operation ended and whether the result was truncated.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Canonical reason an RX operation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RxStopReason {
    /// Operation completed its data collection window (first-byte settle for
    /// `read`, or pattern found / max_bytes reached for `wait_for`).
    DataComplete,
    /// The requested timeout elapsed before the operation could finish normally.
    Timeout,
    /// The literal byte-substring pattern was found in the accumulated buffer.
    MatchFound,
    /// The max_bytes limit was reached before the operation could finish otherwise.
    MaxBufferedBytes,
    /// The underlying serial connection was closed.
    ConnectionClosed,
    /// The MCP request was cancelled by the client.
    Cancelled,
    /// A read error occurred on the serial port.
    ReadError,
    /// The mpsc channel from the pump was closed (pump exited).
    ChannelClosed,
    /// The MCP peer disconnected during streaming.
    PeerDisconnected,
    /// The program buffer budget was insufficient to reserve the requested bytes.
    BudgetExhausted,
}

impl fmt::Display for RxStopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .map_err(|_| fmt::Error)?
            .as_str()
            .ok_or(fmt::Error)?
            .to_string();
        write!(f, "{s}")
    }
}

/// Structured metadata attached to every RX operation result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RxStopMetadata {
    /// Why the operation stopped.
    pub stop_reason: RxStopReason,
    /// `true` when `bytes_returned < bytes_observed` because the operation
    /// capped the returned data (e.g. max_bytes limit exceeded observed data).
    pub truncated: bool,
    /// Total bytes the operation observed from the RX stream before stopping.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_observed: usize,
    /// Bytes actually returned in the result `data` field.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_returned: usize,
}

impl RxStopMetadata {
    pub fn data_complete(bytes_observed: usize, bytes_returned: usize) -> Self {
        Self {
            stop_reason: RxStopReason::DataComplete,
            truncated: bytes_returned < bytes_observed,
            bytes_observed,
            bytes_returned,
        }
    }

    pub fn timeout(bytes_observed: usize) -> Self {
        Self {
            stop_reason: RxStopReason::Timeout,
            truncated: false,
            bytes_observed,
            bytes_returned: bytes_observed,
        }
    }

    pub fn match_found(bytes_observed: usize, bytes_returned: usize) -> Self {
        Self {
            stop_reason: RxStopReason::MatchFound,
            truncated: bytes_returned < bytes_observed,
            bytes_observed,
            bytes_returned,
        }
    }

    pub fn max_buffered_bytes(bytes_observed: usize, bytes_returned: usize) -> Self {
        Self {
            stop_reason: RxStopReason::MaxBufferedBytes,
            truncated: bytes_returned < bytes_observed,
            bytes_observed,
            bytes_returned,
        }
    }

    pub fn connection_closed(bytes_observed: usize, bytes_returned: usize) -> Self {
        Self {
            stop_reason: RxStopReason::ConnectionClosed,
            truncated: bytes_returned < bytes_observed,
            bytes_observed,
            bytes_returned,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            stop_reason: RxStopReason::Cancelled,
            truncated: false,
            bytes_observed: 0,
            bytes_returned: 0,
        }
    }

    pub fn read_error() -> Self {
        Self {
            stop_reason: RxStopReason::ReadError,
            truncated: false,
            bytes_observed: 0,
            bytes_returned: 0,
        }
    }

    pub fn channel_closed() -> Self {
        Self {
            stop_reason: RxStopReason::ChannelClosed,
            truncated: false,
            bytes_observed: 0,
            bytes_returned: 0,
        }
    }

    pub fn peer_disconnected(total_bytes: usize) -> Self {
        Self {
            stop_reason: RxStopReason::PeerDisconnected,
            truncated: false,
            bytes_observed: total_bytes,
            bytes_returned: total_bytes,
        }
    }

    pub fn budget_exhausted() -> Self {
        Self {
            stop_reason: RxStopReason::BudgetExhausted,
            truncated: false,
            bytes_observed: 0,
            bytes_returned: 0,
        }
    }

    pub fn with_bytes(mut self, observed: usize, returned: usize) -> Self {
        self.bytes_observed = observed;
        self.bytes_returned = returned;
        self.truncated = returned < observed;
        self
    }
}
