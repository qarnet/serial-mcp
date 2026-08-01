//! Sliding-window ring buffer with absolute u64 offsets and Notify-based wakeups.
//!
//! [`RxRing`] is the foundation of the RX subsystem. It provides a
//! fixed-capacity circular buffer for received bytes, exposed through absolute
//! stream offsets rather than raw indices. The pump task appends without
//! blocking; reader tasks read by offset and optionally wait for new data
//! via [`wait_for_data`](RxRing::wait_for_data).
//!
//! ## Offset model
//!
//! - `end_offset` is monotonic — total bytes ever appended (never decreases,
//!   even across `clear`).
//! - `start_offset = max(0, end_offset - retained)` where `retained` is the
//!   number of valid bytes currently in the ring (`≤ capacity`). Bytes below
//!   `start_offset` are gone (wrapped out or never stored).
//! - A reader that passes a `cursor < start_offset` observes `bytes_lost` in
//!   the returned slice; the read begins at `start_offset`. Emptiness and gaps
//!   are data, not errors.
//!
//! ## Concurrency
//!
//! `RxRing` is `Send + Sync`. The pump calls `append` without holding async
//! locks. `wait_for_data` drops the internal `Mutex` before awaiting
//! `Notify::notified()`, avoiding `clippy::await_holding_lock`.

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tokio::sync::Notify;

// ── Public types ──────────────────────────────────────────────────────────────

/// A sliding-window ring buffer with absolute u64 stream offsets.
///
/// Shared between a single pump task (calls [`append`](RxRing::append)) and
/// one or more reader tasks (call [`read_from`](RxRing::read_from) and
/// [`wait_for_data`](RxRing::wait_for_data)).
#[allow(dead_code)]
pub(crate) struct RxRing {
    /// Fixed ring capacity in bytes. Immutable after construction.
    capacity: usize,
    /// Mutex-guarded buffer + offset state. Held briefly; never across `.await`.
    inner: Mutex<Inner>,
    /// Wakes readers blocked in `wait_for_data` when new data arrives.
    notify: Notify,
    /// Lifetime total of bytes dropped due to ring wrap. Monotonic.
    bytes_wrapped_total: AtomicU64,
}

/// A slice of data read from the ring at a specific offset range.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct RingSlice {
    /// The data bytes in this slice.
    pub bytes: Vec<u8>,
    /// The actual (clamped) stream offset where this slice begins.
    /// Always `≥ start_offset` at the time of the read.
    pub from_offset: u64,
    /// Cursor value after this slice: `from_offset + bytes.len()`.
    /// Callers should use this as the `cursor` for the next `read_from`.
    pub next_offset: u64,
    /// Number of bytes lost because `cursor < start_offset`.
    /// Zero when `cursor ≥ start_offset`.
    pub bytes_lost: u64,
}

// ── Internal state ───────────────────────────────────────────────────────────

#[allow(dead_code)]
struct Inner {
    /// Ring storage, always exactly `capacity` bytes long.
    buf: Vec<u8>,
    /// First valid stream offset in the buffer. Advances when old data is
    /// wrapped out or when `clear()` is called.
    start_offset: u64,
    /// Total bytes appended since construction. Monotonic — never decreases.
    end_offset: u64,
}

// ── Construction ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
impl RxRing {
    /// Create a new ring buffer.
    ///
    /// # Panics
    ///
    /// Panics if `capacity == 0`. The caller must validate that a positive
    /// buffer size is configured before construction.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RxRing capacity must be > 0");
        Self {
            capacity,
            inner: Mutex::new(Inner {
                buf: vec![0u8; capacity],
                start_offset: 0,
                end_offset: 0,
            }),
            notify: Notify::new(),
            bytes_wrapped_total: AtomicU64::new(0),
        }
    }

    /// Return the ring's fixed capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Return the current start offset (oldest retained byte).
    pub fn start_offset(&self) -> u64 {
        self.inner.lock().expect("ring mutex poisoned").start_offset
    }

    /// Return the current end offset (total bytes appended, monotonic).
    pub fn end_offset(&self) -> u64 {
        self.inner.lock().expect("ring mutex poisoned").end_offset
    }

    /// Return the lifetime total of bytes lost to ring wrap.
    pub fn bytes_wrapped_total(&self) -> u64 {
        self.bytes_wrapped_total.load(Ordering::Relaxed)
    }
}

// ── Append (pump only) ───────────────────────────────────────────────────────

#[allow(dead_code)]
impl RxRing {
    /// Append a chunk of received bytes to the live edge.
    ///
    /// The pump calls this without async locks on the hot path. If the chunk
    /// is larger than the ring's capacity, only the last `capacity` bytes are
    /// retained; older bytes within this chunk are dropped.
    ///
    /// After appending, calls `notify_waiters()` to wake any readers blocked
    /// in [`wait_for_data`](RxRing::wait_for_data).
    pub fn append(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let n = bytes.len() as u64;
        let cap = self.capacity as u64;

        let mut inner = self.inner.lock().expect("ring mutex poisoned");

        let old_end = inner.end_offset;
        let new_end = old_end + n;

        // The new start offset: drop oldest bytes if total retained exceeds capacity.
        let new_start = std::cmp::max(inner.start_offset, new_end.saturating_sub(cap));

        // Track bytes lost to ring wrap (old start vs new start).
        let lost = new_start.saturating_sub(inner.start_offset);
        if lost > 0 {
            self.bytes_wrapped_total.fetch_add(lost, Ordering::Relaxed);
        }

        // Which input bytes survive?
        // Surviving bytes are those at logical positions [new_start, new_end).
        // Bytes in this append live at [old_end, new_end).
        // Intersection: [max(new_start, old_end), new_end).
        let survive_start_logical = std::cmp::max(new_start, old_end);
        let survive_len = (new_end - survive_start_logical) as usize;
        let input_offset = (survive_start_logical - old_end) as usize;

        let src = &bytes[input_offset..input_offset + survive_len];

        // Write into the ring at physical positions.
        let phys_start = (survive_start_logical % cap) as usize;
        write_ring(&mut inner.buf, phys_start, src);

        inner.start_offset = new_start;
        inner.end_offset = new_end;

        drop(inner);
        self.notify.notify_waiters();
    }
}

// ── Read (non-blocking, never fails) ─────────────────────────────────────────

#[allow(dead_code)]
impl RxRing {
    /// Read up to `max` bytes starting at absolute stream offset `cursor`.
    ///
    /// Never blocks and never fails. Returns a [`RingSlice`] describing what
    /// was read (possibly empty). Callers advance their cursor via
    /// `slice.next_offset`.
    ///
    /// # Clamping and gap accounting
    ///
    /// - If `cursor < start_offset`, `bytes_lost = start_offset - cursor` and
    ///   the read starts at `start_offset`.
    /// - If `cursor > end_offset`, returns an empty slice with `from_offset`
    ///   and `next_offset` set to `end_offset` (the current live edge).
    /// - If `max == 0`, returns an empty slice (no bytes copied).
    pub fn read_from(&self, cursor: u64, max: usize) -> RingSlice {
        let inner = self.inner.lock().expect("ring mutex poisoned");
        let start = inner.start_offset;
        let end = inner.end_offset;

        // Gap: cursor is behind the oldest retained byte.
        let bytes_lost = start.saturating_sub(cursor);

        // Clamp the effective read start.
        let from_offset = if cursor < start {
            start
        } else if cursor > end {
            // Caller asked past the live edge; return empty.
            return RingSlice {
                bytes: Vec::new(),
                from_offset: end,
                next_offset: end,
                bytes_lost,
            };
        } else {
            cursor
        };

        if from_offset >= end || max == 0 {
            return RingSlice {
                bytes: Vec::new(),
                from_offset,
                next_offset: from_offset,
                bytes_lost,
            };
        }

        let available = (end - from_offset) as usize;
        let take = max.min(available);

        let cap = self.capacity as u64;
        let phys_start = (from_offset % cap) as usize;

        let bytes = read_ring(&inner.buf, phys_start, take);

        RingSlice {
            bytes,
            from_offset,
            next_offset: from_offset + take as u64,
            bytes_lost,
        }
    }
}

// ── Clear + wait ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
impl RxRing {
    /// Flush all retained data.
    ///
    /// Sets `start_offset = end_offset`. The ring appears empty until
    /// subsequent `append` calls advance `end_offset` further. `end_offset`
    /// remains monotonic — it is never reset.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("ring mutex poisoned");
        inner.start_offset = inner.end_offset;
    }

    /// Wait until `end_offset > after` (i.e. new data has been appended past
    /// `after`).
    ///
    /// Returns immediately if the condition is already satisfied.
    ///
    /// Uses `tokio::sync::Notify`. The lock is dropped before `.await` to
    /// avoid `clippy::await_holding_lock`.
    pub async fn wait_for_data(&self, after: u64) {
        loop {
            // Register interest BEFORE checking the condition. This is the
            // canonical Notify pattern: if notify_waiters fires between
            // notified() creation and the lock acquisition, the future still
            // resolves immediately because Tokio stores the permit.
            let notified = self.notify.notified();
            {
                let inner = self.inner.lock().expect("ring mutex poisoned");
                if inner.end_offset > after {
                    return;
                }
            }
            // Lock dropped; safe to await.
            notified.await;
        }
    }
}

// ── Helpers: write / read with wrap ─────────────────────────────────────────

/// Copy `src` into `buf` starting at physical index `phys`, wrapping around
/// the buffer boundary as needed.
#[allow(dead_code)]
fn write_ring(buf: &mut [u8], phys: usize, src: &[u8]) {
    let cap = buf.len();
    let end = phys + src.len();
    if end <= cap {
        buf[phys..end].copy_from_slice(src);
    } else {
        let first = cap - phys;
        buf[phys..].copy_from_slice(&src[..first]);
        buf[..end - cap].copy_from_slice(&src[first..]);
    }
}

/// Copy `take` bytes from `buf` starting at physical index `phys`, wrapping
/// around the buffer boundary as needed.
#[allow(dead_code)]
fn read_ring(buf: &[u8], phys: usize, take: usize) -> Vec<u8> {
    let cap = buf.len();
    let end = phys + take;
    if end <= cap {
        buf[phys..end].to_vec()
    } else {
        let first = cap - phys;
        let mut out = Vec::with_capacity(take);
        out.extend_from_slice(&buf[phys..]);
        out.extend_from_slice(&buf[..take - first]);
        out
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Construction ──────────────────────────────────────────────────────

    #[test]
    fn new_ring_has_zero_offsets() {
        let ring = RxRing::new(16);
        assert_eq!(ring.start_offset(), 0);
        assert_eq!(ring.end_offset(), 0);
        assert_eq!(ring.capacity(), 16);
        let slice = ring.read_from(0, 10);
        assert!(slice.bytes.is_empty());
        assert_eq!(slice.from_offset, 0);
        assert_eq!(slice.next_offset, 0);
        assert_eq!(slice.bytes_lost, 0);
    }

    #[test]
    #[should_panic(expected = "RxRing capacity must be > 0")]
    fn new_zero_capacity_panics() {
        let _ring = RxRing::new(0);
    }

    // ── Basic append + read ───────────────────────────────────────────────

    #[test]
    fn append_advances_end_offset() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        assert_eq!(ring.start_offset(), 0);
        assert_eq!(ring.end_offset(), 3);
    }

    #[test]
    fn read_from_returns_appended_bytes() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        let slice = ring.read_from(0, 10);
        assert_eq!(slice.bytes, b"abc");
        assert_eq!(slice.from_offset, 0);
        assert_eq!(slice.next_offset, 3);
        assert_eq!(slice.bytes_lost, 0);
    }

    #[test]
    fn read_from_advances_cursor_via_next_offset() {
        let ring = RxRing::new(16);
        ring.append(b"abcdef");

        let s1 = ring.read_from(0, 3);
        assert_eq!(s1.bytes, b"abc");
        assert_eq!(s1.next_offset, 3);

        let s2 = ring.read_from(3, 3);
        assert_eq!(s2.bytes, b"def");
        assert_eq!(s2.next_offset, 6);
    }

    #[test]
    fn read_from_respects_max() {
        let ring = RxRing::new(16);
        ring.append(b"abcdef");
        let slice = ring.read_from(0, 3);
        assert_eq!(slice.bytes.len(), 3);
        assert_eq!(slice.bytes, b"abc");
        assert_eq!(slice.next_offset, 3);
    }

    #[test]
    fn read_from_zero_max_returns_empty() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        let slice = ring.read_from(0, 0);
        assert!(slice.bytes.is_empty());
        assert_eq!(slice.from_offset, 0);
        assert_eq!(slice.next_offset, 0);
        assert_eq!(slice.bytes_lost, 0);
    }

    #[test]
    fn read_from_clamps_cursor_above_end_offset() {
        let ring = RxRing::new(16);
        ring.append(b"abc"); // end_offset = 3
        let slice = ring.read_from(100, 10);
        assert!(slice.bytes.is_empty());
        // from_offset / next_offset set to end_offset (current live edge).
        assert_eq!(slice.from_offset, 3);
        assert_eq!(slice.next_offset, 3);
        assert_eq!(slice.bytes_lost, 0);
    }

    // ── Append empty ──────────────────────────────────────────────────────

    #[test]
    fn append_empty_is_noop() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        ring.append(b"");
        assert_eq!(ring.start_offset(), 0);
        assert_eq!(ring.end_offset(), 3);
        let slice = ring.read_from(0, 10);
        assert_eq!(slice.bytes, b"abc");
    }

    // ── Wrap: append past capacity drops oldest ───────────────────────────

    #[test]
    fn append_wraps_and_drops_oldest() {
        // capacity 4
        let ring = RxRing::new(4);
        ring.append(b"abcd"); // full: start=0, end=4
        assert_eq!(ring.start_offset(), 0);
        assert_eq!(ring.end_offset(), 4);

        ring.append(b"efgh"); // wrap: start=4, end=8
        assert_eq!(ring.start_offset(), 4);
        assert_eq!(ring.end_offset(), 8);

        // Read from start_offset: should see b"efgh"
        let slice = ring.read_from(4, 4);
        assert_eq!(slice.bytes, b"efgh");
        assert_eq!(slice.from_offset, 4);
        assert_eq!(slice.next_offset, 8);
        assert_eq!(slice.bytes_lost, 0);

        // Read from cursor 0 (before start): gap.
        let old = ring.read_from(0, 4);
        assert_eq!(old.bytes_lost, 4);
        assert_eq!(old.from_offset, 4); // clamped to start
        assert_eq!(old.bytes, b"efgh");
    }

    #[test]
    fn append_larger_than_capacity_keeps_only_tail() {
        // capacity 4, append 8 bytes — only last 4 survive.
        let ring = RxRing::new(4);
        ring.append(b"abcdefgh"); // 8 bytes
        assert_eq!(ring.start_offset(), 4);
        assert_eq!(ring.end_offset(), 8);

        let slice = ring.read_from(4, 4);
        assert_eq!(slice.bytes, b"efgh");
        assert_eq!(slice.bytes_lost, 0);
    }

    #[test]
    fn append_multiple_wraps_track_correctly() {
        // Append multiple chunks across several wraps.
        let ring = RxRing::new(4);
        ring.append(b"ab"); // start=0, end=2
        ring.append(b"cd"); // start=0, end=4 (full)
        ring.append(b"ef"); // start=2, end=6
        ring.append(b"gh"); // start=4, end=8
        ring.append(b"ij"); // start=6, end=10

        let slice = ring.read_from(6, 4);
        assert_eq!(slice.bytes, b"ghij");
        assert_eq!(slice.from_offset, 6);
        assert_eq!(slice.next_offset, 10);
        assert_eq!(slice.bytes_lost, 0);
    }

    #[test]
    fn read_from_wrap_reads_across_buffer_boundary() {
        // Fill ring so that a read must span the buffer wrap.
        // capacity=4, append 7 bytes → only last 4 survive ("defg"),
        // placed straddling the buffer boundary.
        let ring = RxRing::new(4);
        ring.append(b"abcdefg");
        // total=7, start=3, end=7
        // Surviving bytes: input[3..7] = "defg"
        // Physical: d at pos 3, e at pos 0, f at pos 1, g at pos 2
        // buf = [e, f, g, d]
        // Logical range: 3→d(pos3), 4→e(pos0), 5→f(pos1), 6→g(pos2)

        // Read from start (3) with max 4 crosses phys boundary (3→0→1→2).
        let slice = ring.read_from(3, 4);
        assert_eq!(slice.bytes, b"defg");
        assert_eq!(slice.from_offset, 3);
        assert_eq!(slice.next_offset, 7);
    }

    #[test]
    fn append_larger_than_capacity_wraps_and_writes_correctly() {
        // capacity 4, append 7 bytes → only last 4 survive (positions 3..7).
        let ring = RxRing::new(4);
        ring.append(b"abcdefg"); // 7 bytes
        assert_eq!(ring.start_offset(), 3);
        assert_eq!(ring.end_offset(), 7);

        let slice = ring.read_from(3, 4);
        assert_eq!(slice.bytes, b"defg");
        assert_eq!(slice.bytes_lost, 0);
    }

    // ── Gap accounting ────────────────────────────────────────────────────

    #[test]
    fn read_from_gap_accounting_when_cursor_below_start() {
        let ring = RxRing::new(4);
        ring.append(b"abcd"); // start=0, end=4
        ring.append(b"efgh"); // start=4, end=8

        // cursor=2 < start=4 → bytes_lost=2, clamped to start=4
        let slice = ring.read_from(2, 4);
        assert_eq!(slice.bytes_lost, 2);
        assert_eq!(slice.from_offset, 4);
        assert_eq!(slice.bytes, b"efgh");
        assert_eq!(slice.next_offset, 8);
    }

    #[test]
    fn read_from_no_gap_when_cursor_equals_start() {
        let ring = RxRing::new(4);
        ring.append(b"abcd");
        ring.append(b"efgh"); // start=4, end=8

        let slice = ring.read_from(4, 4);
        assert_eq!(slice.bytes_lost, 0);
        assert_eq!(slice.from_offset, 4);
    }

    // ── Clear ─────────────────────────────────────────────────────────────

    #[test]
    fn clear_resets_retained_start_catches_up_to_end() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        assert_eq!(ring.start_offset(), 0);
        assert_eq!(ring.end_offset(), 3);

        ring.clear();
        // start catches up to end; end stays monotonic.
        assert_eq!(ring.start_offset(), 3);
        assert_eq!(ring.end_offset(), 3);

        // Read from 0 now: cursor < start → gap + empty.
        let slice = ring.read_from(0, 10);
        assert_eq!(slice.bytes_lost, 3); // start - cursor = 3 - 0
        assert!(slice.bytes.is_empty());
        assert_eq!(slice.from_offset, 3);
        assert_eq!(slice.next_offset, 3);
    }

    #[test]
    fn clear_then_append_resumes_at_end_offset() {
        let ring = RxRing::new(16);
        ring.append(b"abc"); // end=3
        ring.clear(); // start=3, end=3

        ring.append(b"xy"); // end=5
        assert_eq!(ring.start_offset(), 3);
        assert_eq!(ring.end_offset(), 5);

        let slice = ring.read_from(3, 2);
        assert_eq!(slice.bytes, b"xy");
        assert_eq!(slice.from_offset, 3);
        assert_eq!(slice.next_offset, 5);
        assert_eq!(slice.bytes_lost, 0);
    }

    // ── Cursor at end_offset (caught up) ─────────────────────────────────

    #[test]
    fn read_from_caught_up_reader_gets_empty() {
        let ring = RxRing::new(16);
        ring.append(b"abc");
        let slice = ring.read_from(3, 10);
        assert!(slice.bytes.is_empty());
        assert_eq!(slice.from_offset, 3);
        assert_eq!(slice.next_offset, 3);
        assert_eq!(slice.bytes_lost, 0);
    }

    // ── Notify wakeups ────────────────────────────────────────────────────

    #[tokio::test]
    async fn wait_for_data_wakes_on_append() {
        let ring = std::sync::Arc::new(RxRing::new(16));
        let ring_clone = ring.clone();

        // Spawn a task that blocks on wait_for_data(0).
        let handle = tokio::spawn(async move {
            ring_clone.wait_for_data(0).await;
        });

        // Brief yield so the task parks.
        tokio::task::yield_now().await;

        // Append — should wake the parked task.
        ring.append(b"x");
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_data_returns_immediately_when_data_already_past_after() {
        let ring = RxRing::new(16);
        ring.append(b"abc"); // end=3

        // after=2 < end=3 → returns immediately (no park).
        ring.wait_for_data(2).await;
    }

    #[tokio::test]
    async fn wait_for_data_spurious_wakeup_safe() {
        // Even if wait_for_data wakes spuriously (Notify can do this),
        // the loop re-checks end_offset and re-parks correctly.
        let ring = std::sync::Arc::new(RxRing::new(16));
        let ring_clone = ring.clone();

        let handle = tokio::spawn(async move {
            ring_clone.wait_for_data(0).await;
        });

        tokio::task::yield_now().await;

        // Notify without appending data — spurious.
        ring.notify.notify_waiters();

        // The task should re-check and park again, NOT return.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert!(!handle.is_finished());

        // Now append real data.
        ring.append(b"x");
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_data_multiple_waiters_wake() {
        let ring = std::sync::Arc::new(RxRing::new(16));

        let r1 = {
            let r = ring.clone();
            tokio::spawn(async move { r.wait_for_data(0).await })
        };
        let r2 = {
            let r = ring.clone();
            tokio::spawn(async move { r.wait_for_data(0).await })
        };

        tokio::task::yield_now().await;

        ring.append(b"hello");
        r1.await.unwrap();
        r2.await.unwrap();
    }

    // ── Send + Sync ───────────────────────────────────────────────────────

    #[test]
    fn ring_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<RxRing>();
        assert_sync::<RxRing>();
    }

    // ── End offset monotonicity ───────────────────────────────────────────

    #[test]
    fn end_offset_never_decreases_across_clear() {
        let ring = RxRing::new(16);
        ring.append(b"hello");
        let before = ring.end_offset();
        ring.clear();
        assert_eq!(ring.end_offset(), before);
    }

    /// bytes_wrapped_total accumulates lifetime wrap-loss across multiple wraps
    /// and is NOT reset by clear().
    #[test]
    fn bytes_wrapped_total_accumulates_across_wraps_and_survives_clear() {
        let ring = RxRing::new(4);
        assert_eq!(ring.bytes_wrapped_total(), 0);

        // Fill past capacity: 8 bytes into cap=4 → start=4, end=8, 4 lost.
        ring.append(b"abcdefgh");
        assert_eq!(ring.bytes_wrapped_total(), 4);

        // read_from with cursor below start reports per-slice bytes_lost,
        // but does NOT increment bytes_wrapped_total (which is wrap-only).
        let s1 = ring.read_from(2, 4);
        assert_eq!(s1.bytes_lost, 2);
        assert_eq!(ring.bytes_wrapped_total(), 4); // unchanged by read

        // Append more past capacity to trigger another wrap.
        ring.append(b"ijklmnop");
        // start 4→12, end 8→16, drop positions 4..12 (8 bytes lost on wrap).
        assert_eq!(ring.bytes_wrapped_total(), 12);

        // clear() does NOT reset the lifetime accumulator.
        ring.clear();
        assert_eq!(ring.bytes_wrapped_total(), 12);

        // Append without wrap — no additional loss.
        ring.append(b"qr");
        assert_eq!(ring.bytes_wrapped_total(), 12);
    }

    // ── Proptest: modeled append/read vs reference stream ─────────────────

    mod proptest_tests {
        use proptest::prelude::*;

        proptest! {
            /// Model a sequence of append/read operations against a reference
            /// stream (all bytes ever appended). Assert offset arithmetic and
            /// data integrity after every operation.
            #[test]
            fn rx_ring_append_read_preserves_stream_and_offset_arithmetic(
                ops in ops_strategy()
            ) {
                let capacity = 4; // small — forces frequent wraps
                let ring = super::RxRing::new(capacity);
                let mut total_appended: u64 = 0;
                let mut stream: Vec<u8> = Vec::new();

                for op in ops {
                    match op {
                        Op::Append(data) => {
                            ring.append(&data);
                            stream.extend_from_slice(&data);
                            total_appended += data.len() as u64;

                            // After append, offsets must match the model.
                            let expected_start = total_appended.saturating_sub(capacity as u64);
                            assert_eq!(ring.end_offset(), total_appended,
                                "end_offset mismatch after append");
                            assert_eq!(ring.start_offset(), expected_start,
                                "start_offset mismatch after append");
                        }
                        Op::Read { cursor, max } => {
                            let current_start = ring.start_offset();
                            let current_end = ring.end_offset();

                            // Expected gap.
                            let expected_bytes_lost = current_start.saturating_sub(cursor);

                            // Expected from_offset.
                            let expected_from = if cursor < current_start {
                                current_start
                            } else if cursor > current_end {
                                current_end
                            } else {
                                cursor
                            };

                            // Expected bytes from the reference stream.
                            let available = if expected_from < current_end {
                                (current_end - expected_from) as usize
                            } else {
                                0
                            };
                            let take = max.min(available);
                            let model_start = expected_from as usize;
                            let model_end = model_start + take;
                            let expected_bytes: &[u8] = if model_start < stream.len() {
                                let end = model_end.min(stream.len());
                                &stream[model_start..end]
                            } else {
                                &[]
                            };

                            let slice = ring.read_from(cursor, max);

                            assert_eq!(slice.bytes_lost, expected_bytes_lost,
                                "bytes_lost mismatch");
                            assert_eq!(slice.from_offset, expected_from,
                                "from_offset mismatch");

                            if cursor > current_end {
                                assert!(slice.bytes.is_empty(),
                                    "expected empty for cursor beyond end");
                                assert_eq!(slice.next_offset, current_end);
                            } else {
                                assert_eq!(slice.bytes, expected_bytes,
                                    "data mismatch");
                                assert_eq!(slice.next_offset, expected_from + take as u64,
                                    "next_offset mismatch");
                            }
                        }
                    }
                }
            }
        }

        #[derive(Debug, Clone)]
        enum Op {
            Append(Vec<u8>),
            Read { cursor: u64, max: usize },
        }

        fn ops_strategy() -> impl Strategy<Value = Vec<Op>> {
            let append = prop::collection::vec(0u8..=255u8, 0..32).prop_map(Op::Append);
            let read = (0u64..32, 0usize..32).prop_map(|(cursor, max)| Op::Read { cursor, max });
            prop::collection::vec(
                prop::strategy::Union::new(vec![append.boxed(), read.boxed()]),
                1..100,
            )
        }
    }
}
