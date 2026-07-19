pub const MIN_READ_BYTES: usize = 1;
pub const MAX_READ_BYTES: usize = 1024 * 1024; // 1 MiB
pub const MIN_STREAM_CHUNK_BYTES: usize = 1;
pub const MAX_STREAM_CHUNK_BYTES: usize = 64 * 1024; // 64 KiB
pub const MAX_TIMEOUT_MS: u64 = 5 * 60 * 1000; // 5 min
pub const MIN_POLL_INTERVAL_MS: u64 = 10;
pub const MAX_WRITE_BYTES: usize = 1024 * 1024; // 1 MiB

// Buffer budget defaults
pub const DEFAULT_MAX_PROGRAM_BUFFERED_BYTES: usize = 1024 * 1024 * 1024; // 1 GiB
pub const DEFAULT_MAX_TOOL_BUFFERED_BYTES: usize = 1024 * 1024; // 1 MiB

// RX ring buffer defaults (per-connection capture ring)
pub const DEFAULT_RX_BUFFER_SIZE: usize = 256 * 1024; // 256 KiB
pub const MAX_RX_BUFFER_SIZE: usize = 16 * 1024 * 1024; // 16 MiB per-connection ceiling
