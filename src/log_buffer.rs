//! Per-connection in-memory event log with ring-buffer semantics.
//!
//! Stores typed [`LogEvent`] entries with millisecond-precision Unix
//! timestamps. When the buffer reaches `capacity`, oldest entries are
//! dropped. Accessible via tools (`get_log`, `clear_log`, `export_log`)
//! and a resource (`serial://connections/{id}/log`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// ---- Event types -----------------------------------------------------------

/// A single log event with timestamp, direction, and event data.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogEntry {
    /// Milliseconds since Unix epoch.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timestamp_ms: u64,
    /// `"rx"`, `"tx"`, or `null` for non-directional events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The typed event payload.
    pub event: LogEvent,
}

/// Typed event payloads stored in the log buffer.
///
/// The `#[serde(tag = "event")]` attribute ensures each entry carries a
/// `"event"` field naming the variant, producing clean JSONL output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LogEvent {
    /// RX data chunk received from the device.
    RxData {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        bytes: usize,
    },
    /// TX data chunk sent to the device.
    TxData {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        bytes: usize,
    },
    /// Connection opened.
    Open,
    /// Connection closed (explicitly by user).
    Close,
    /// Port disconnected (fatal I/O error).
    Disconnect { error: String },
    /// Reconnect attempt started.
    ReconnectStart {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        attempt: u32,
    },
    /// Reconnect succeeded.
    ReconnectSuccess,
    /// Reconnect attempt failed.
    ReconnectFailed {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        attempt: u32,
        error: String,
    },
    /// Max reconnect attempts exhausted.
    ReconnectExhausted,
    /// Match found in read/subscribe.
    MatchFound { pattern: String, mode: String },
    /// Data truncated (bytes_returned < bytes_observed).
    Truncated {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        observed: usize,
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        returned: usize,
    },
    /// Notification dropped (encoding error or peer disconnect).
    NotificationDropped { reason: String },
    /// Read operation started.
    ReadStarted,
    /// Read operation completed.
    ReadCompleted {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        bytes: usize,
    },
    /// Write operation started.
    WriteStarted,
    /// Write operation completed.
    WriteCompleted {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        bytes: usize,
    },
    /// Subscribe started.
    SubscribeStarted,
    /// Subscribe stopped.
    SubscribeStopped { reason: String },
    /// Generic error.
    Error { message: String },
}

// ---- Log buffer ------------------------------------------------------------

/// Thread-safe, bounded ring buffer of timestamped log events.
#[derive(Debug)]
pub struct LogBuffer {
    events: Mutex<VecDeque<LogEntry>>,
    capacity: usize,
    enabled: bool,
}

impl LogBuffer {
    /// Create a new log buffer with the given capacity and enabled flag.
    pub fn new(capacity: usize, enabled: bool) -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            enabled,
        }
    }

    /// Return whether logging is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Return the configured capacity (maximum number of events).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Record a typed event with an optional direction.
    pub fn record(&self, direction: Option<&str>, event: LogEvent) {
        if !self.enabled {
            return;
        }
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let entry = LogEntry {
            timestamp_ms,
            direction: direction.map(str::to_string),
            event,
        };
        let mut events = self.events.lock().expect("log mutex poisoned");
        events.push_back(entry);
        while events.len() > self.capacity {
            events.pop_front();
        }
    }

    /// Return a snapshot of all entries in the buffer.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        let events = self.events.lock().expect("log mutex poisoned");
        events.iter().cloned().collect()
    }

    /// Clear all entries from the buffer.
    pub fn clear(&self) {
        let mut events = self.events.lock().expect("log mutex poisoned");
        events.clear();
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(1024, false)
    }
}

/// Typed wrapper to share across the server.
impl LogBuffer {
    pub fn new_shared(capacity: usize, enabled: bool) -> Arc<Self> {
        Arc::new(Self::new(capacity, enabled))
    }
}

// Helper shorthand for recording common events.
impl LogBuffer {
    pub fn rx_data(&self, bytes: usize) {
        self.record(Some("rx"), LogEvent::RxData { bytes });
    }

    pub fn tx_data(&self, bytes: usize) {
        self.record(Some("tx"), LogEvent::TxData { bytes });
    }

    pub fn opened(&self) {
        self.record(None, LogEvent::Open);
    }

    pub fn closed(&self) {
        self.record(None, LogEvent::Close);
    }

    pub fn match_found(&self, pattern: &str, mode: &str) {
        self.record(
            None,
            LogEvent::MatchFound {
                pattern: pattern.to_string(),
                mode: mode.to_string(),
            },
        );
    }

    pub fn truncated(&self, observed: usize, returned: usize) {
        self.record(None, LogEvent::Truncated { observed, returned });
    }

    pub fn notification_dropped(&self, reason: &str) {
        self.record(
            None,
            LogEvent::NotificationDropped {
                reason: reason.to_string(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_records_and_snapshots() {
        let log = LogBuffer::new(10, true);
        log.rx_data(5);
        log.tx_data(3);
        let snap = log.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap[0].timestamp_ms > 0);
    }

    #[test]
    fn log_buffer_disabled_does_not_record() {
        let log = LogBuffer::new(10, false);
        log.rx_data(5);
        assert!(log.snapshot().is_empty());
    }

    #[test]
    fn log_buffer_evicts_when_capacity_exceeded() {
        let log = LogBuffer::new(3, true);
        for i in 0..5 {
            log.rx_data(i);
        }
        let snap = log.snapshot();
        assert_eq!(snap.len(), 3);
        // Oldest (0,1) should be evicted; 2,3,4 remain
        if let LogEvent::RxData { bytes } = &snap[0].event {
            assert_eq!(*bytes, 2);
        }
    }

    #[test]
    fn log_buffer_clear_empties() {
        let log = LogBuffer::new(10, true);
        log.rx_data(1);
        log.tx_data(2);
        log.clear();
        assert!(log.snapshot().is_empty());
    }

    #[test]
    fn log_entry_serialization_is_jsonl_compatible() {
        let log = LogBuffer::new(10, true);
        log.rx_data(42);
        let snap = log.snapshot();
        let json = serde_json::to_string(&snap[0]).unwrap();
        // Verify the "event" tag field is present
        assert!(json.contains("\"event\":\"rx_data\""));
        assert!(json.contains("\"bytes\":42"));
        assert!(json.contains("\"direction\":\"rx\""));
    }
}
