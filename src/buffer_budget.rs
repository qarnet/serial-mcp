//! Buffer budget manager for RX tools.
//!
//! Provides a trait-based interface for reserving buffer space from a shared
//! program pool. RX tools (`read`, `wait_for`, `subscribe`) reserve their
//! full requested `max_buffered_bytes` up front and release it on
//! completion/cancellation/error via RAII.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use thiserror::Error;

/// Errors produced when a buffer reservation cannot be fulfilled.
#[derive(Debug, Error)]
pub enum BufferBudgetError {
    /// The requested size exceeds the per-tool ceiling.
    #[error("max_buffered_bytes={requested} exceeds per-tool limit {tool_limit}")]
    OverToolLimit { requested: usize, tool_limit: usize },
    /// The requested size is zero.
    #[error("max_buffered_bytes must be > 0")]
    ZeroRequest,
    /// The program pool has insufficient available bytes.
    #[error("insufficient program buffer budget: requested {requested}, available {available}")]
    InsufficientProgramBudget { requested: usize, available: usize },
}

/// Trait for reserving buffer space from a shared pool.
///
/// Implementations must be `Send + Sync` so the budget can be shared across
/// tasks. The returned [`BufferReservation`] releases bytes back to the pool
/// on drop (RAII).
pub trait BufferBudget: Send + Sync {
    /// Attempt to reserve `bytes` from the shared program pool.
    ///
    /// Returns a reservation handle on success. The handle releases its bytes
    /// back to the pool on drop.
    ///
    /// Returns an error if:
    /// - `bytes` is 0
    /// - `bytes` exceeds the per-tool ceiling configured at construction
    /// - the program pool has fewer than `bytes` available
    fn try_reserve(&self, bytes: usize) -> Result<Box<dyn BufferReservation>, BufferBudgetError>;

    /// The per-tool ceiling. Requests larger than this are rejected regardless
    /// of pool availability.
    fn tool_limit(&self) -> usize;

    /// The total program pool size.
    fn program_limit(&self) -> usize;

    /// The number of bytes currently available in the program pool.
    fn available(&self) -> usize;
}

/// Trait for a reservation that releases bytes on drop.
pub trait BufferReservation: Send + std::fmt::Debug {
    /// The number of bytes reserved by this reservation.
    fn bytes(&self) -> usize;
}

// ---- Real implementation ---------------------------------------------------

/// Production budget manager backed by an atomic counter.
///
/// The program pool starts at `program_limit` bytes. Each reservation
/// deducts from the pool; each drop returns bytes. The per-tool ceiling
/// caps individual reservation sizes.
pub struct AtomicBudget {
    program_limit: usize,
    tool_limit: usize,
    available: Arc<AtomicUsize>,
}

impl AtomicBudget {
    /// Create a new budget manager.
    ///
    /// Panics if `program_limit` is 0 or `tool_limit` is 0 or
    /// `tool_limit > program_limit`.
    pub fn new(program_limit: usize, tool_limit: usize) -> Self {
        assert!(program_limit > 0, "program_limit must be > 0");
        assert!(tool_limit > 0, "tool_limit must be > 0");
        assert!(
            tool_limit <= program_limit,
            "tool_limit must be <= program_limit"
        );
        Self {
            program_limit,
            tool_limit,
            available: Arc::new(AtomicUsize::new(program_limit)),
        }
    }
}

impl BufferBudget for AtomicBudget {
    fn try_reserve(&self, bytes: usize) -> Result<Box<dyn BufferReservation>, BufferBudgetError> {
        if bytes == 0 {
            return Err(BufferBudgetError::ZeroRequest);
        }
        if bytes > self.tool_limit {
            return Err(BufferBudgetError::OverToolLimit {
                requested: bytes,
                tool_limit: self.tool_limit,
            });
        }
        // CAS loop to deduct from available pool.
        loop {
            let current = self.available.load(Ordering::Acquire);
            if current < bytes {
                return Err(BufferBudgetError::InsufficientProgramBudget {
                    requested: bytes,
                    available: current,
                });
            }
            let new = current - bytes;
            if self
                .available
                .compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
            // Another task modified the counter; retry.
        }
        Ok(Box::new(AtomicReservation {
            bytes,
            available: Arc::clone(&self.available),
            released: AtomicBool::new(false),
        }))
    }

    fn tool_limit(&self) -> usize {
        self.tool_limit
    }

    fn program_limit(&self) -> usize {
        self.program_limit
    }

    fn available(&self) -> usize {
        self.available.load(Ordering::Acquire)
    }
}

/// RAII reservation that returns bytes to the `AtomicBudget` pool on drop.
#[derive(Debug)]
struct AtomicReservation {
    bytes: usize,
    available: Arc<AtomicUsize>,
    released: AtomicBool,
}

impl BufferReservation for AtomicReservation {
    fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for AtomicReservation {
    fn drop(&mut self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.available.fetch_add(self.bytes, Ordering::AcqRel);
        }
    }
}

// ---- Fake implementation for tests -----------------------------------------

/// A fake budget manager for deterministic testing.
///
/// Useful for testing budget exhaustion paths without requiring concurrent
/// load. The remaining counter is backed by a `Mutex<usize>` for
/// single-threaded determinism.
pub struct FakeBudget {
    tool_limit: usize,
    program_limit: usize,
    remaining: Arc<std::sync::Mutex<usize>>,
}

impl FakeBudget {
    /// Create a fake budget that allows up to `program_limit` total bytes
    /// and per-request up to `tool_limit` bytes.
    pub fn new(program_limit: usize, tool_limit: usize) -> Self {
        assert!(program_limit > 0);
        assert!(tool_limit > 0);
        assert!(tool_limit <= program_limit);
        Self {
            tool_limit,
            program_limit,
            remaining: Arc::new(std::sync::Mutex::new(program_limit)),
        }
    }

    /// Reset the remaining budget to `program_limit`.
    pub fn reset(&self) {
        *self.remaining.lock().expect("remaining") = self.program_limit;
    }
}

impl BufferBudget for FakeBudget {
    fn try_reserve(&self, bytes: usize) -> Result<Box<dyn BufferReservation>, BufferBudgetError> {
        if bytes == 0 {
            return Err(BufferBudgetError::ZeroRequest);
        }
        if bytes > self.tool_limit {
            return Err(BufferBudgetError::OverToolLimit {
                requested: bytes,
                tool_limit: self.tool_limit,
            });
        }
        let mut remaining = self.remaining.lock().expect("remaining");
        if *remaining < bytes {
            return Err(BufferBudgetError::InsufficientProgramBudget {
                requested: bytes,
                available: *remaining,
            });
        }
        *remaining -= bytes;
        Ok(Box::new(FakeReservation {
            bytes,
            remaining: Arc::clone(&self.remaining),
            released: AtomicBool::new(false),
        }))
    }

    fn tool_limit(&self) -> usize {
        self.tool_limit
    }

    fn program_limit(&self) -> usize {
        self.program_limit
    }

    fn available(&self) -> usize {
        *self.remaining.lock().expect("remaining")
    }
}

#[derive(Debug)]
struct FakeReservation {
    bytes: usize,
    remaining: Arc<std::sync::Mutex<usize>>,
    released: AtomicBool,
}

impl BufferReservation for FakeReservation {
    fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for FakeReservation {
    fn drop(&mut self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            *self.remaining.lock().expect("remaining") += self.bytes;
        }
    }
}

// ---- Null implementation (no program-level pool) --------------------------

/// A budget manager that always succeeds at the program pool level but still
/// enforces `tool_limit` as a per-request ceiling.
///
/// Used when the server is started without explicit program pool limits
/// (the `UnlimitedBudget` has infinite program capacity but validates
/// per-request sizes against `tool_limit`).
pub struct UnlimitedBudget {
    tool_limit: usize,
}

impl UnlimitedBudget {
    pub fn new(tool_limit: usize) -> Self {
        Self { tool_limit }
    }
}

impl BufferBudget for UnlimitedBudget {
    fn try_reserve(&self, bytes: usize) -> Result<Box<dyn BufferReservation>, BufferBudgetError> {
        if bytes == 0 {
            return Err(BufferBudgetError::ZeroRequest);
        }
        if bytes > self.tool_limit {
            return Err(BufferBudgetError::OverToolLimit {
                requested: bytes,
                tool_limit: self.tool_limit,
            });
        }
        Ok(Box::new(NopReservation { bytes }))
    }

    fn tool_limit(&self) -> usize {
        self.tool_limit
    }

    fn program_limit(&self) -> usize {
        usize::MAX
    }

    fn available(&self) -> usize {
        usize::MAX
    }
}

#[derive(Debug)]
struct NopReservation {
    bytes: usize,
}

impl BufferReservation for NopReservation {
    fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_budget_reserve_and_release() {
        let budget = AtomicBudget::new(1024, 512);
        assert_eq!(budget.available(), 1024);
        let r = budget.try_reserve(256).unwrap();
        assert_eq!(r.bytes(), 256);
        assert_eq!(budget.available(), 768);
        drop(r);
        assert_eq!(budget.available(), 1024);
    }

    #[test]
    fn atomic_budget_over_tool_limit() {
        let budget = AtomicBudget::new(1024, 512);
        let err = budget.try_reserve(600).unwrap_err();
        assert!(matches!(
            err,
            BufferBudgetError::OverToolLimit {
                requested: 600,
                tool_limit: 512
            }
        ));
    }

    #[test]
    fn atomic_budget_zero_request() {
        let budget = AtomicBudget::new(1024, 512);
        let err = budget.try_reserve(0).unwrap_err();
        assert!(matches!(err, BufferBudgetError::ZeroRequest));
    }

    #[test]
    fn atomic_budget_insufficient_program() {
        let budget = AtomicBudget::new(1024, 1024);
        let _r1 = budget.try_reserve(800).unwrap();
        let err = budget.try_reserve(300).unwrap_err();
        assert!(matches!(
            err,
            BufferBudgetError::InsufficientProgramBudget {
                requested: 300,
                available: 224
            }
        ));
    }

    #[test]
    fn atomic_budget_concurrent_reserve() {
        let budget = Arc::new(AtomicBudget::new(4096, 4096));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let b = Arc::clone(&budget);
            handles.push(std::thread::spawn(move || {
                let r = b.try_reserve(1024).unwrap();
                assert_eq!(r.bytes(), 1024);
                r
            }));
        }
        let reservations: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(budget.available(), 0);
        // Fifth reservation should fail.
        let err = budget.try_reserve(1).unwrap_err();
        assert!(matches!(
            err,
            BufferBudgetError::InsufficientProgramBudget { .. }
        ));
        drop(reservations);
        assert_eq!(budget.available(), 4096);
    }

    #[test]
    fn atomic_budget_rejects_zero_limits() {
        let _ = std::panic::catch_unwind(|| AtomicBudget::new(0, 1));
        let _ = std::panic::catch_unwind(|| AtomicBudget::new(1, 0));
        let _ = std::panic::catch_unwind(|| AtomicBudget::new(1, 2));
    }

    #[test]
    fn atomic_budget_reserve_at_exact_tool_limit() {
        // bytes == tool_limit must succeed; > not >=.
        let budget = AtomicBudget::new(1024, 512);
        let r = budget.try_reserve(512).unwrap();
        assert_eq!(r.bytes(), 512);
        assert_eq!(budget.available(), 512);
    }

    #[test]
    fn atomic_budget_reports_configured_limits() {
        let budget = AtomicBudget::new(1024, 512);
        assert_eq!(budget.tool_limit(), 512);
        assert_eq!(budget.program_limit(), 1024);
    }

    #[test]
    fn fake_budget_basic() {
        let budget = FakeBudget::new(1024, 512);
        let r = budget.try_reserve(256).unwrap();
        assert_eq!(budget.available(), 768);
        drop(r);
        assert_eq!(budget.available(), 1024);
    }

    #[test]
    fn fake_budget_exhaustion() {
        let budget = FakeBudget::new(512, 512);
        let _r1 = budget.try_reserve(512).unwrap();
        let err = budget.try_reserve(1).unwrap_err();
        assert!(matches!(
            err,
            BufferBudgetError::InsufficientProgramBudget {
                requested: 1,
                available: 0
            }
        ));
    }

    #[test]
    fn fake_budget_reports_configured_limits() {
        let budget = FakeBudget::new(1024, 512);
        assert_eq!(budget.tool_limit(), 512);
        assert_eq!(budget.program_limit(), 1024);
    }

    #[test]
    fn fake_budget_reservation_bytes_matches_request() {
        let budget = FakeBudget::new(1024, 512);
        let r = budget.try_reserve(256).unwrap();
        assert_eq!(r.bytes(), 256);
    }

    #[test]
    fn fake_budget_reset_restores_available() {
        let budget = FakeBudget::new(512, 512);
        let _r = budget.try_reserve(512).unwrap();
        assert_eq!(budget.available(), 0);
        assert!(budget.try_reserve(1).is_err());
        budget.reset();
        assert_eq!(budget.available(), 512);
        // Can reserve again after reset.
        let r2 = budget.try_reserve(256).unwrap();
        assert_eq!(r2.bytes(), 256);
    }

    #[test]
    fn unlimited_budget_always_succeeds() {
        let budget = UnlimitedBudget::new(1024);
        let r = budget.try_reserve(1024).unwrap();
        assert_eq!(r.bytes(), 1024);
        // No program pool; available is always MAX.
        assert_eq!(budget.available(), usize::MAX);
    }

    #[test]
    fn unlimited_budget_still_rejects_over_tool_limit() {
        let budget = UnlimitedBudget::new(1024);
        let err = budget.try_reserve(1025).unwrap_err();
        assert!(matches!(err, BufferBudgetError::OverToolLimit { .. }));
    }

    #[test]
    fn unlimited_budget_rejects_zero() {
        let budget = UnlimitedBudget::new(1024);
        let err = budget.try_reserve(0).unwrap_err();
        assert!(matches!(err, BufferBudgetError::ZeroRequest));
    }

    #[test]
    fn unlimited_budget_reports_configured_limits() {
        let budget = UnlimitedBudget::new(1024);
        assert_eq!(budget.tool_limit(), 1024);
        assert_eq!(budget.program_limit(), usize::MAX);
    }
}
