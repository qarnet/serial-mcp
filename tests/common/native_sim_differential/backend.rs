//! Endpoint ownership for native-vs-fixture differential scenarios.

use std::time::Duration;

use anyhow::{ensure, Context, Result};

use super::super::device_fixture::core::Action;
use super::super::device_fixture::{DeviceFixture, DeviceFixtureConfig, DevicePeer};
use super::super::firmware::NativeSimFirmware;

pub const BOOT_BANNER: &[u8] = b"serial-mcp test firmware ready\r\n";
pub const PONG_RESPONSE: &[u8] = b"pong\r\n";
pub const JSONOUT_RESPONSE: &[u8] = b"{\"sensor\":\"temp\",\"value\":25.5,\"unit\":\"C\"}\r\n{\"sensor\":\"humidity\",\"value\":60,\"unit\":\"%\"}\r\n{\"sensor\":\"pressure\",\"value\":1013.25,\"unit\":\"hPa\"}\r\n";
pub const NDJSON_PRESET_JSON_FRAMES_RESPONSE: &[u8] = b"{\"a\":1}\n\n{\"b\":2}\n";
pub const NDJSON_PRESET_SKIPS_EMPTY_LINES_RESPONSE: &[u8] =
    b"{\"a\":1}\n\n\n{\"b\":2}\n   \n{\"c\":3}\n";

const SPAM_CHUNK_SIZE: usize = 256;
const SPAM_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Native,
    Fixture,
}

pub enum DifferentialEndpoint {
    Native(NativeSimFirmware),
    Fixture(DeviceFixture),
}

impl DifferentialEndpoint {
    pub async fn spawn(kind: BackendKind) -> Result<Self> {
        match kind {
            BackendKind::Native => Ok(Self::Native(NativeSimFirmware::spawn().await?)),
            BackendKind::Fixture => Ok(Self::Fixture(
                DeviceFixture::spawn(CompatibilityPeer::default(), DeviceFixtureConfig::default())
                    .await?,
            )),
        }
    }

    pub fn port_path(&self) -> Result<String> {
        match self {
            Self::Native(firmware) => Ok(firmware.pty_path().to_owned()),
            Self::Fixture(fixture) => fixture
                .port_path()
                .to_str()
                .map(str::to_owned)
                .context("fixture PTY path was not valid UTF-8"),
        }
    }

    /// Explicit bounded teardown. Fixture task abort is a failure because this
    /// helper's compatibility peer must shut down cleanly.
    pub async fn shutdown(self) -> Result<()> {
        match self {
            Self::Native(firmware) => {
                let _ = firmware.shutdown_and_join().await?;
                Ok(())
            }
            Self::Fixture(fixture) => {
                let report = fixture.shutdown().await?;
                ensure!(
                    !report.task_aborted,
                    "fixture differential endpoint required task abort during shutdown: {:?}",
                    report.snapshot
                );
                Ok(())
            }
        }
    }
}

/// Narrow differential peer. It deliberately lives here instead of changing
/// the general fixture default, and models only bytes required by current
/// executable batches.
#[derive(Debug, Default)]
struct CompatibilityPeer {
    framing_on: bool,
    trace_on: bool,
    trace_sequence: u8,
    ack_enabled: bool,
    ack_sequence: u32,
    armed_delay: Option<Duration>,
}

fn derive_spam_stream(count: usize) -> Vec<u8> {
    let start = format!("spam start count={count} delay=10\r\n");
    let completion = format!("Spam complete: {count} bytes sent\r\n");
    let mut stream = Vec::with_capacity(start.len() + count + completion.len());
    stream.extend_from_slice(start.as_bytes());

    let mut state = 0x1234_5678u32;
    for _ in 0..count {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        stream.push(b"0123456789abcdef"[(state & 0x0f) as usize]);
    }

    stream.extend_from_slice(completion.as_bytes());
    stream
}

pub(crate) fn spam_stream(count: usize) -> Vec<u8> {
    derive_spam_stream(count)
}

fn spam_actions(count: usize) -> Vec<Action> {
    let stream = derive_spam_stream(count);
    let start_len = format!("spam start count={count} delay=10\r\n").len();
    let completion_len = format!("Spam complete: {count} bytes sent\r\n").len();
    let data_end = stream.len() - completion_len;
    let mut actions = vec![Action::Emit(stream[..start_len].to_vec())];
    for chunk in stream[start_len..data_end].chunks(SPAM_CHUNK_SIZE) {
        actions.push(Action::Delay(SPAM_DELAY));
        actions.push(Action::Emit(chunk.to_vec()));
    }
    actions.push(Action::Emit(stream[data_end..].to_vec()));
    actions
}

impl DevicePeer for CompatibilityPeer {
    fn on_start(&mut self) -> Vec<Action> {
        vec![Action::Emit(BOOT_BANNER.to_vec())]
    }

    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(delay) = self.armed_delay.take() {
            actions.push(Action::Delay(delay));
        }
        if self.ack_enabled {
            actions.push(Action::Emit(
                format!("ack {}\r\n", self.ack_sequence).into_bytes(),
            ));
            self.ack_sequence = self.ack_sequence.wrapping_add(1);
        }
        actions.extend(match command {
            b"ack on" => {
                self.ack_enabled = true;
                self.ack_sequence = 0;
                vec![Action::Emit(b"ack on\r\n".to_vec())]
            }
            b"ack off" => {
                self.ack_enabled = false;
                vec![Action::Emit(b"ack off\r\n".to_vec())]
            }
            b"arm_cmd 1000" => {
                self.armed_delay = Some(Duration::from_millis(1000));
                vec![Action::Emit(b"arm_cmd delay=1000\r\n".to_vec())]
            }
            b"sendraw hex C0706F6E67C0" => {
                vec![Action::Emit(vec![0xc0, b'p', b'o', b'n', b'g', 0xc0])]
            }
            b"sendraw hex C0DB41C0" => vec![Action::Emit(vec![0xc0, 0xdb, 0x41, 0xc0])],
            b"sendraw hex 0005706F6E6700" => {
                vec![Action::Emit(vec![0x00, 0x05, b'p', b'o', b'n', b'g', 0x00])]
            }
            b"sendraw hex 7B2261223A317D0A0A7B2262223A327D0A" => {
                vec![Action::Emit(NDJSON_PRESET_JSON_FRAMES_RESPONSE.to_vec())]
            }
            b"sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A" => {
                vec![Action::Emit(
                    NDJSON_PRESET_SKIPS_EMPTY_LINES_RESPONSE.to_vec(),
                )]
            }
            b"framing on" => {
                self.framing_on = true;
                vec![Action::Emit(b"framing on\r\n".to_vec())]
            }
            b"trace on" => {
                self.trace_on = true;
                self.trace_sequence = 0;
                vec![Action::Emit(b"trace on\r\n".to_vec())]
            }
            b"ping" => {
                let mut actions = Vec::new();
                if self.trace_on {
                    for byte in b"ping\r\n" {
                        actions.push(Action::Emit(
                            format!("RX[{}]=0x{byte:02x}\r\n", self.trace_sequence).into_bytes(),
                        ));
                        self.trace_sequence = self.trace_sequence.wrapping_add(1);
                    }
                }
                if self.framing_on {
                    actions.push(Action::Emit(b"LINE len=4 data=\"ping\"\r\n".to_vec()));
                    actions.push(Action::Emit(b"LINE len=4 data=\"ping\"\r\n".to_vec()));
                }
                actions.push(Action::Emit(PONG_RESPONSE.to_vec()));
                actions
            }
            b"jsonout" => vec![Action::Emit(JSONOUT_RESPONSE.to_vec())],
            b"write cmd 1 ping" => {
                vec![Action::Emit(b"ack 1 exec>ping\r\npong\r\n".to_vec())]
            }
            b"spam 1024 hex" => spam_actions(1024),
            b"spam 512 hex" => spam_actions(512),
            _ => vec![Action::Emit(b"ERROR\r\n".to_vec())],
        });
        actions
    }
}
