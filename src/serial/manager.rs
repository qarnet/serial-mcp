//! The multi-connection registry: `ConnectionManager`, its private registry
//! state, duplicate-port lookup, reconnect-task supervision, and
//! manager-only tests. Depends on the `config` and `connection` siblings.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::{Result, SerialError};

use super::config::{ConnectionConfig, ConnectionState, ConnectionSummary};
use super::connection::SerialConnection;

/// Registry of currently open serial connections, indexed by an opaque
/// connection id. Rejects opening the same port twice.
#[derive(Debug, Default)]
pub struct ConnectionManager {
    state: Mutex<ConnectionRegistry>,
}

#[derive(Debug, Default)]
struct ConnectionRegistry {
    connections: HashMap<String, Arc<SerialConnection>>,
    opening_ports: HashSet<String>,
    closing_ports: HashSet<String>,
    reconnect_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new connection and store it. Returns the new connection id.
    pub async fn open(&self, config: ConnectionConfig) -> Result<String> {
        let port = config.port.clone();
        {
            let mut state = self.state.lock().await;
            if let Some(connection) = find_connection_by_port(&state.connections, &port) {
                return Err(SerialError::PortAlreadyOpen {
                    port,
                    connection_id: Some(connection.id().to_string()),
                    name: connection.name().map(str::to_string),
                });
            }
            if state.opening_ports.contains(&port) || state.closing_ports.contains(&port) {
                return Err(SerialError::PortAlreadyOpening(port));
            }
            state.opening_ports.insert(port.clone());
        }

        let opened = SerialConnection::open(config).await;
        let mut state = self.state.lock().await;
        state.opening_ports.remove(&port);
        let connection = Arc::new(opened?);
        let id = connection.id().to_string();
        state.connections.insert(id.clone(), connection);
        Ok(id)
    }

    /// Insert an already-built [`SerialConnection`] (typically one backed
    /// by an in-memory loopback) into the registry. Honours the same
    /// port-uniqueness invariant as [`Self::open`].
    ///
    /// Exposed for integration tests that want to drive the MCP surface
    /// against a fake connection without going through the OS serial layer.
    pub async fn insert(&self, connection: SerialConnection) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(existing) = find_connection_by_port(&state.connections, connection.port()) {
            return Err(SerialError::PortAlreadyOpen {
                port: connection.port().to_string(),
                connection_id: Some(existing.id().to_string()),
                name: existing.name().map(str::to_string),
            });
        }
        if state.opening_ports.contains(connection.port())
            || state.closing_ports.contains(connection.port())
        {
            return Err(SerialError::PortAlreadyOpening(
                connection.port().to_string(),
            ));
        }
        let id = connection.id().to_string();
        state.connections.insert(id.clone(), Arc::new(connection));
        Ok(id)
    }

    /// Remove a connection, cancel in-flight operations, flush RX, and close
    /// the underlying port before allowing a reopen.
    pub async fn close(&self, id: &str) -> Result<()> {
        let connection = {
            let mut state = self.state.lock().await;
            let connection = state
                .connections
                .remove(id)
                .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))?;
            state.closing_ports.insert(connection.port().to_string());
            connection
        };

        let port = connection.port().to_string();
        // Abort any running reconnect task for this connection.
        {
            let mut state = self.state.lock().await;
            if let Some(handle) = state.reconnect_tasks.remove(id) {
                handle.abort();
            }
        }
        connection.log().closed();
        let result = connection.close().await;

        self.state.lock().await.closing_ports.remove(&port);
        result
    }

    /// Look up an existing connection by id.
    pub async fn get(&self, id: &str) -> Result<Arc<SerialConnection>> {
        self.state
            .lock()
            .await
            .connections
            .get(id)
            .cloned()
            .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))
    }

    /// Return all currently-registered connections with their ids.
    pub async fn list_all(&self) -> Vec<(String, Arc<SerialConnection>)> {
        self.state
            .lock()
            .await
            .connections
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Start a background reconnect task for the given connection.
    /// The task retries `reconnect()` with exponential backoff,
    /// respecting the connection's `ReconnectPolicy`. On success,
    /// restarts the RX pump via `rx_sessions`.
    pub async fn start_reconnect(
        &self,
        id: &str,
        rx_sessions: Arc<crate::rx_session::RxSessionManager>,
    ) {
        let conn = match self.get(id).await {
            Ok(c) => c,
            Err(_) => return,
        };
        let policy = conn.reconnect_policy.lock().expect("poisoned").clone();
        if !policy.enabled {
            return;
        }
        // Avoid spawning a duplicate task. Prune finished handles first.
        {
            let mut state = self.state.lock().await;
            state.reconnect_tasks.retain(|_, h| !h.is_finished());
            if state.reconnect_tasks.contains_key(id) {
                return;
            }
        }

        let id_owned = id.to_string();
        let conn_clone = Arc::clone(&conn);
        let handle = tokio::spawn(async move {
            let mut delay_ms = policy.initial_delay_ms;
            let mut attempts: u32 = 0;
            loop {
                // Check if still disconnected / not cancelled.
                let state = conn_clone.state();
                if state == ConnectionState::Open || state == ConnectionState::Closed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                match conn_clone.reconnect().await {
                    Ok(()) => {
                        // Reset attempt counter after successful reconnect.
                        conn_clone.reset_reconnect_attempts();
                        // Restart the RX pump so data flows again.
                        if let Some(session) = rx_sessions.get(&id_owned).await {
                            session.ensure_pump_running();
                        }
                        break;
                    }
                    Err(_e) => {
                        attempts += 1;
                        if policy.max_attempts > 0 && attempts >= policy.max_attempts {
                            conn_clone
                                .log()
                                .record(None, crate::log_buffer::LogEvent::ReconnectExhausted);
                            break;
                        }
                        // Exponential backoff with cap.
                        delay_ms = ((delay_ms as f64) * policy.backoff_multiplier)
                            .min(policy.max_delay_ms as f64)
                            as u64;
                    }
                }
            }
            // Task completes: handle stays in reconnect_tasks; supervisor
            // prunes finished handles on its next poll.
        });

        let mut state = self.state.lock().await;
        state.reconnect_tasks.insert(id.to_string(), handle);
    }

    /// Cancel a running reconnect task for the given connection.
    pub async fn cancel_reconnect(&self, id: &str) {
        let mut state = self.state.lock().await;
        if let Some(handle) = state.reconnect_tasks.remove(id) {
            handle.abort();
        }
    }

    /// Number of currently open connections.
    pub async fn count(&self) -> usize {
        self.state.lock().await.connections.len()
    }

    /// Lightweight snapshot of all currently-open connections. Cheap because
    /// it only clones the id + port pair, not the underlying IO.
    pub async fn list_open(&self) -> Vec<ConnectionSummary> {
        self.state
            .lock()
            .await
            .connections
            .values()
            .map(|c| c.summary())
            .collect()
    }
}

fn find_connection_by_port<'a>(
    connections: &'a HashMap<String, Arc<SerialConnection>>,
    port: &str,
) -> Option<&'a Arc<SerialConnection>> {
    connections.values().find(|c| c.port() == port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::test_support::{loopback_connection, loopback_connection_with_config};
    use crate::serial::{DataBits, FlowControl, Parity, StopBits};

    #[tokio::test]
    async fn manager_rejects_duplicate_port() {
        let mgr = ConnectionManager::new();
        let (c1, _p1) = loopback_connection("port-a");
        mgr.insert(c1).await.unwrap();
        let (c2, _p2) = loopback_connection("port-a");
        let err = mgr.insert(c2).await.unwrap_err();
        assert!(matches!(err, SerialError::PortAlreadyOpen { .. }));
    }

    #[tokio::test]
    async fn manager_duplicate_port_error_includes_owner_metadata() {
        let mgr = ConnectionManager::new();
        let (c1, _peer_a) = loopback_connection_with_config(ConnectionConfig {
            port: "port-owner".into(),
            name: Some("console".into()),
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
            rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
            max_buffered_bytes: 32768,
            poll_interval_ms: 200,
        });
        let owner_id = mgr.insert(c1).await.unwrap();

        let (c2, _p2) = loopback_connection("port-owner");
        let err = mgr.insert(c2).await.unwrap_err();
        match err {
            SerialError::PortAlreadyOpen {
                port,
                connection_id,
                name,
            } => {
                assert_eq!(port, "port-owner");
                assert_eq!(connection_id.as_deref(), Some(owner_id.as_str()));
                assert_eq!(name.as_deref(), Some("console"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manager_close_then_get_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let (c, _p) = loopback_connection("port-z");
        let id = mgr.insert(c).await.unwrap();
        mgr.close(&id).await.unwrap();
        let err = mgr.get(&id).await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
    }

    #[tokio::test]
    async fn manager_get_unknown_id_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let err = mgr.get("does-not-exist").await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
    }
}
