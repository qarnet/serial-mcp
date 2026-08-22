//! Pure state used by the PTY device fixture.
//!
//! Keeping command assembly, script validation, and queue accounting free of
//! OS I/O makes boundary behavior deterministic and cheap to test.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{bail, Result};

/// One bounded device-side action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Emit(Vec<u8>),
    EmitChunks(Vec<Vec<u8>>),
    Delay(Duration),
    Silence(Duration),
    Malformed(Vec<u8>),
    Saturate(Vec<u8>),
    Close,
    Crash(i32),
}

impl Action {
    fn supplied_bytes(&self) -> usize {
        match self {
            Self::Emit(bytes) | Self::Malformed(bytes) | Self::Saturate(bytes) => bytes.len(),
            Self::EmitChunks(chunks) => chunks.iter().map(Vec::len).sum(),
            Self::Delay(_) | Self::Silence(_) | Self::Close | Self::Crash(_) => 0,
        }
    }
}

/// Per-script limits. These prevent peers from creating unbounded work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptLimits {
    pub max_actions: usize,
    pub max_supplied_bytes: usize,
}

impl ScriptLimits {
    pub fn validate(self, actions: &[Action]) -> Result<()> {
        if actions.len() > self.max_actions {
            bail!(
                "device script has {} actions; limit is {}",
                actions.len(),
                self.max_actions
            );
        }
        let supplied_bytes = actions.iter().try_fold(0usize, |total, action| {
            total.checked_add(action.supplied_bytes()).ok_or_else(|| {
                anyhow::anyhow!("device script supplied-byte count overflowed usize")
            })
        })?;
        if supplied_bytes > self.max_supplied_bytes {
            bail!(
                "device script supplies {supplied_bytes} bytes; limit is {}",
                self.max_supplied_bytes
            );
        }
        Ok(())
    }
}

/// CR/LF command assembler with a hard pending-input bound.
#[derive(Debug)]
pub struct InputAssembler {
    pending: Vec<u8>,
    max_pending_bytes: usize,
}

impl InputAssembler {
    pub fn new(max_pending_bytes: usize) -> Result<Self> {
        if max_pending_bytes == 0 {
            bail!("device input limit must be greater than zero");
        }
        Ok(Self {
            pending: Vec::new(),
            max_pending_bytes,
        })
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        let next_len = self
            .pending
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| anyhow::anyhow!("device input length overflowed usize"))?;
        if next_len > self.max_pending_bytes {
            bail!(
                "device input has {next_len} pending bytes; limit is {}",
                self.max_pending_bytes
            );
        }
        self.pending.extend_from_slice(bytes);

        let mut commands = Vec::new();
        while let Some(index) = self
            .pending
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
        {
            let command = self.pending.drain(..index).collect::<Vec<_>>();
            self.pending.remove(0);
            while matches!(self.pending.first(), Some(b'\r' | b'\n')) {
                self.pending.remove(0);
            }
            if !command.is_empty() {
                commands.push(command);
            }
        }
        Ok(commands)
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Queue-full behavior selected explicitly by each fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePolicy {
    /// Accept bytes that fit and count remaining bytes as dropped.
    DropNew,
    /// Accept bytes that fit and return remaining bytes to caller for retry.
    BlockProducer,
}

/// Cumulative queue accounting exposed through fixture readiness snapshots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub accepted: usize,
    pub dropped: usize,
    pub drained: usize,
}

/// Result of one bounded enqueue attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnqueueReport {
    pub accepted: usize,
    pub dropped: usize,
    pub blocked: usize,
}

/// Byte-bounded output queue. Kernel PTY buffering happens after this queue.
#[derive(Debug)]
pub struct OutputQueue {
    bytes: VecDeque<u8>,
    capacity: usize,
    max_drain_chunk: usize,
    policy: QueuePolicy,
    held: bool,
    stats: QueueStats,
}

impl OutputQueue {
    pub fn new(capacity: usize, max_drain_chunk: usize, policy: QueuePolicy) -> Result<Self> {
        if capacity == 0 {
            bail!("device output capacity must be greater than zero");
        }
        if max_drain_chunk == 0 {
            bail!("device output chunk size must be greater than zero");
        }
        Ok(Self {
            bytes: VecDeque::new(),
            capacity,
            max_drain_chunk,
            policy,
            held: false,
            stats: QueueStats::default(),
        })
    }

    pub fn enqueue(&mut self, bytes: &[u8]) -> EnqueueReport {
        let available = self.capacity.saturating_sub(self.bytes.len());
        let accepted = available.min(bytes.len());
        self.bytes.extend(&bytes[..accepted]);
        self.stats.accepted = self.stats.accepted.saturating_add(accepted);
        let remainder = bytes.len().saturating_sub(accepted);
        let (dropped, blocked) = match self.policy {
            QueuePolicy::DropNew => (remainder, 0),
            QueuePolicy::BlockProducer => (0, remainder),
        };
        self.stats.dropped = self.stats.dropped.saturating_add(dropped);
        EnqueueReport {
            accepted,
            dropped,
            blocked,
        }
    }

    pub fn next_chunk(&self) -> Vec<u8> {
        if self.held {
            return Vec::new();
        }
        self.bytes
            .iter()
            .take(self.max_drain_chunk)
            .copied()
            .collect()
    }

    pub fn commit_drain(&mut self, count: usize) -> Result<()> {
        if count > self.bytes.len() {
            bail!(
                "cannot drain {count} bytes from {} queued bytes",
                self.bytes.len()
            );
        }
        self.bytes.drain(..count);
        self.stats.drained = self.stats.drained.saturating_add(count);
        Ok(())
    }

    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    pub fn held(&self) -> bool {
        self.held
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn stats(&self) -> QueueStats {
        self.stats
    }
}
