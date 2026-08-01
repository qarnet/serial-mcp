//! Process-wide persistent capture store (Phase 6).
//!
//! Establishes the containment, symlink, quota, atomicity, and lifecycle
//! policy required before any future continuous raw capture feature. No
//! continuous capture tool exists yet — this phase only hardens
//! `export_log`.
//!
//! Policy summary:
//!
//! - **Disabled by default.** The store only persists when the server
//!   starts with an explicit absolute `--capture-dir`. There is no fallback
//!   to cwd, OS config, or temp directories.
//! - **Flat portable filename contract.** Exports are written as one
//!   portable `.jsonl` filename inside the root. No subdirectories, no
//!   separators, no traversal, no Windows-reserved stems, no internal
//!   `.serial-mcp-` reserved prefix (which owns the lock and temp files).
//! - **Root and symlink policy.** The configured root must be absolute,
//!   existing, a directory, and not itself a symlink; it is canonicalized
//!   once at startup. The advisory lock path must be a regular non-symlink
//!   file. Managed-name symlink entries inside the root are rejected and
//!   never followed. The configured root and its ancestors remain the
//!   operator-controlled trust boundary — this phase deliberately does not
//!   defend against an operator replacing trusted root ancestors while the
//!   server runs (portable std/tempfile/fs2 APIs cannot provide
//!   directory-handle-relative guarantees).
//! - **Quotas.** Per-file, total-byte, and file-count quotas are enforced
//!   from a fresh scan of direct children under a process-local async mutex
//!   and a cross-process advisory lock, so cooperating serial-mcp processes
//!   sharing a root cannot exceed the quotas.
//! - **Atomic no-clobber commit.** Bytes are written to a same-root temp
//!   file (reserved internal prefix), `sync_all`-ed, then committed with
//!   `persist_noclobber`. A PRE-commit failure leaves no final file and
//!   changes no existing capture. A temp file may survive a process crash;
//!   this phase never silently treats it as committed and never deletes
//!   arbitrary files. On Unix the root directory is synced after commit; a
//!   POST-commit root-sync failure is reported as a `durability_warning` on
//!   the successful result (the file is committed and counted; it is never
//!   deleted). Windows crash-durability of the rename is a documented
//!   portable limitation (no root sync, so no warning is produced).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use tokio::sync::Mutex;

/// Name of the advisory cross-process lock file inside the capture root.
pub const CAPTURE_LOCK_FILE: &str = ".serial-mcp-captures.lock";

/// Reserved internal prefix for same-root temp files. The quota scan
/// ignores entries with this prefix, and no committed capture filename may
/// contain it.
pub const CAPTURE_TEMP_PREFIX: &str = ".serial-mcp-capture-";

/// The shared reserved prefix for all internal `.serial-mcp-*` entries
/// (lock file, temp files). Rejected inside user filenames so the managed
/// namespace stays unambiguous.
pub const CAPTURE_RESERVED_PREFIX: &str = ".serial-mcp-";

/// Maximum portable capture filename length in bytes/characters, including
/// the `.jsonl` suffix.
pub const MAX_CAPTURE_FILENAME_LEN: usize = 120;

/// Required case-sensitive suffix for committed capture files.
pub const CAPTURE_FILENAME_SUFFIX: &str = ".jsonl";

/// Error surfaced whenever persistent capture is not configured. Exact
/// public message asserted by the behavior tests.
pub const CAPTURE_DISABLED_ERROR: &str =
    "Persistent capture is disabled; start serial-mcp with --capture-dir <absolute-directory>";

/// Quota limits for a capture root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureLimits {
    /// Per-file byte ceiling (the bounded JSONL snapshot must fit).
    pub max_file_bytes: u64,
    /// Total bytes across all committed managed files.
    pub max_total_bytes: u64,
    /// Total number of committed managed files.
    pub max_files: usize,
}

/// Outcome of a successful [`CaptureStore::write_new`] commit.
///
/// A `Some` [`Self::durability_warning`] marks a POST-commit condition: the
/// file is committed and counts toward quota, but crash-durability of the
/// rename could not be confirmed on this filesystem (the only post-persist
/// fallible step is the root-directory sync on Unix). Pre-commit failures
/// are `Err` and never create a final file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWriteResult {
    /// Canonical absolute path of the committed file inside the root.
    pub path: PathBuf,
    /// Exact bytes committed (post-quota, post-commit).
    pub bytes_written: u64,
    /// Managed file count including this file.
    pub files_used: usize,
    /// Total managed bytes including this file.
    pub total_bytes_used: u64,
    /// Post-commit durability warning (root-dir sync failure on Unix), or
    /// `None` when durability was confirmed (or is not applicable, e.g.
    /// Windows, where the limitation is documented). The commit itself
    /// succeeded either way; the committed file is never deleted.
    pub durability_warning: Option<String>,
}

/// Process-wide capture store, shared by every stdio/HTTP handler.
///
/// `root == None` marks a disabled store: `write_new` always errors with
/// [`CAPTURE_DISABLED_ERROR`] and no file or lock work happens.
#[derive(Debug)]
pub struct CaptureStore {
    /// Canonical absolute root; `None` = disabled.
    root: Option<PathBuf>,
    limits: CaptureLimits,
    /// Process-local serialization. The advisory root lock serializes
    /// cooperating processes sharing the same root.
    lock: Mutex<()>,
}

impl CaptureStore {
    /// A disabled store that never touches disk. Library/builder default.
    pub fn disabled() -> Self {
        Self {
            root: None,
            limits: CaptureLimits {
                max_file_bytes: 0,
                max_total_bytes: 0,
                max_files: 0,
            },
            lock: Mutex::new(()),
        }
    }

    /// Open a persistent store rooted at `root`.
    ///
    /// Validates: absolute path, existing directory, not itself a symlink
    /// (via `symlink_metadata`), limits all `> 0` with `max_file_bytes <=
    /// max_total_bytes`. The lock path must be a regular non-symlink file
    /// and the advisory lock must provably work (locked, then released).
    /// The root is canonicalized once here; every transaction runs against
    /// the canonical root.
    pub fn open(root: PathBuf, limits: CaptureLimits) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err(format!(
                "capture dir must be an absolute path: {}",
                root.display()
            ));
        }
        if limits.max_file_bytes == 0 || limits.max_total_bytes == 0 || limits.max_files == 0 {
            return Err("capture limits must all be > 0".to_string());
        }
        if limits.max_file_bytes > limits.max_total_bytes {
            return Err(format!(
                "capture per-file limit ({}) must be <= total limit ({})",
                limits.max_file_bytes, limits.max_total_bytes
            ));
        }

        let meta = std::fs::symlink_metadata(&root)
            .map_err(|e| format!("capture dir {}: {e}", root.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "capture dir must not be a symlink: {}",
                root.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!(
                "capture dir is not a directory: {}",
                root.display()
            ));
        }

        let canonical = root
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize capture dir {}: {e}", root.display()))?;

        // Prove the advisory lock works at startup: acquire and release.
        let lock_file = acquire_root_lock(&canonical)?;
        fs2::FileExt::unlock(&lock_file)
            .map_err(|e| format!("cannot release capture lock: {e}"))?;

        Ok(Self {
            root: Some(canonical),
            limits,
            lock: Mutex::new(()),
        })
    }

    /// Whether persistent capture is configured.
    pub fn is_enabled(&self) -> bool {
        self.root.is_some()
    }

    /// Per-file byte ceiling for the bounded snapshot.
    pub fn max_file_bytes(&self) -> u64 {
        self.limits.max_file_bytes
    }

    /// The canonical capture root, when enabled.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Atomically commit `bytes` as a new file named `requested_name`
    /// inside the root.
    ///
    /// Order: disabled/name validation, per-file quota (checked `usize` →
    /// `u64` conversion) BEFORE the process-local mutex or any blocking
    /// work, then blocking work under the exclusive advisory root lock:
    /// fresh scan of managed files, no-clobber destination check, count and
    /// total quotas (checked arithmetic), same-root temp write + `sync_all`,
    /// `persist_noclobber`, root-dir sync on Unix. Pre-commit failure
    /// creates no final file and changes no existing capture; a post-commit
    /// root-sync failure is reported as [`CaptureWriteResult::durability_warning`]
    /// on a successful commit (the file is committed and counted; it is
    /// never deleted).
    pub async fn write_new(
        &self,
        requested_name: String,
        bytes: Vec<u8>,
    ) -> Result<CaptureWriteResult, String> {
        let Some(root) = self.root.clone() else {
            return Err(CAPTURE_DISABLED_ERROR.to_string());
        };
        validate_capture_filename(&requested_name)?;
        // Per-file quota rejection runs before any mutex/spawn_blocking
        // work; `commit_new_file` re-checks under the lock (defense in
        // depth — limits are Copy, so the in-lock check can never disagree).
        let bytes_len = u64::try_from(bytes.len())
            .map_err(|_| "capture file size does not fit a u64 counter".to_string())?;
        if bytes_len > self.limits.max_file_bytes {
            return Err(format!(
                "capture file exceeds per-file quota: {bytes_len} bytes > --capture-max-file-bytes {}",
                self.limits.max_file_bytes
            ));
        }
        let limits = self.limits;
        let _guard = self.lock.lock().await;
        tokio::task::spawn_blocking(move || commit_new_file(&root, &requested_name, &bytes, limits))
            .await
            .map_err(|e| format!("capture commit task failed: {e}"))?
    }
}

/// Validate a portable capture filename per the Phase 6 contract:
///
/// - ASCII, 1..=[`MAX_CAPTURE_FILENAME_LEN`] characters including `.jsonl`
/// - starts alphanumeric; remaining chars only alphanumeric, `.`, `_`, `-`
/// - ends `.jsonl` case-sensitively; no `/` or `\`; not `.`/`..`; no
///   absolute/root/prefix components or nested path (the charset guarantees
///   flat single-component names)
/// - no internal `.serial-mcp-` reserved prefix
/// - Windows-reserved stems rejected case-insensitively (`CON`, `PRN`,
///   `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, including with extension)
pub fn validate_capture_filename(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("capture filename is empty".to_string());
    }
    if name.len() > MAX_CAPTURE_FILENAME_LEN {
        return Err(format!(
            "capture filename exceeds {MAX_CAPTURE_FILENAME_LEN} characters (including '.jsonl')"
        ));
    }
    if !name.is_ascii() {
        return Err("capture filename must be ASCII".to_string());
    }
    if name == "." || name == ".." {
        return Err("capture filename must not be '.' or '..'".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("capture filename must not contain path separators".to_string());
    }
    if !name.ends_with(CAPTURE_FILENAME_SUFFIX) {
        return Err(format!(
            "capture filename must end with '{CAPTURE_FILENAME_SUFFIX}'"
        ));
    }
    if name.contains(CAPTURE_RESERVED_PREFIX) {
        return Err(format!(
            "capture filename must not contain the reserved internal prefix '{CAPTURE_RESERVED_PREFIX}'"
        ));
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err("capture filename must start with an alphanumeric character".to_string());
    }
    for &b in &bytes[1..] {
        if !(b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-') {
            return Err(
                "capture filename may only contain alphanumeric, '.', '_', and '-'".to_string(),
            );
        }
    }
    // Windows-reserved device stems (case-insensitive), with extension:
    // the stem is everything before the first '.'.
    let stem = name.split('.').next().unwrap_or(name);
    if is_windows_reserved_stem(stem) {
        return Err(format!(
            "capture filename uses reserved stem '{stem}' (Windows device names are rejected)"
        ));
    }
    Ok(())
}

fn is_windows_reserved_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let reserved_numeric = |prefix: &str| {
        let Some(tail) = upper.strip_prefix(prefix) else {
            return false;
        };
        tail.len() == 1 && tail.as_bytes()[0].is_ascii_digit() && tail.as_bytes()[0] != b'0'
    };
    reserved_numeric("COM") || reserved_numeric("LPT")
}

/// Whether `name` is a committed-managed capture filename: a valid portable
/// `.jsonl` regular file name. Lock and temp entries are excluded by their
/// reserved prefix before this is consulted.
fn is_managed_capture_filename(name: &str) -> bool {
    validate_capture_filename(name).is_ok()
}

/// Acquire the exclusive advisory lock on the capture root. Rejects a
/// symlink or non-regular lock path every time (an operator or attacker who
/// can replace the root contents must not be able to redirect the lock).
fn acquire_root_lock(root: &Path) -> Result<File, String> {
    let lock_path = root.join(CAPTURE_LOCK_FILE);
    match std::fs::symlink_metadata(&lock_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "capture lock path is a symlink: {}",
                    lock_path.display()
                ));
            }
            if !meta.is_file() {
                return Err(format!(
                    "capture lock path is not a regular file: {}",
                    lock_path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot stat capture lock path {}: {e}",
                lock_path.display()
            ))
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open capture lock file {}: {e}", lock_path.display()))?;
    file.lock_exclusive()
        .map_err(|e| format!("cannot lock capture root {}: {e}", lock_path.display()))?;
    Ok(file)
}

/// Current usage of committed managed files in the root, from a fresh scan
/// of direct children only. Unknown/orphan entries are ignored (never
/// deleted); symlink entries with managed names are rejected (never
/// followed). Directory/special entries with managed names are not counted
/// — the no-clobber check on a new destination rejects them instead.
#[derive(Debug)]
struct RootUsage {
    files: usize,
    total_bytes: u64,
}

fn scan_managed_usage(root: &Path) -> Result<RootUsage, String> {
    let entries = std::fs::read_dir(root)
        .map_err(|e| format!("cannot scan capture dir {}: {e}", root.display()))?;
    let mut files: usize = 0;
    let mut total_bytes: u64 = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read capture dir entry: {e}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            // Non-UTF-8 names are not managed files; ignore, never delete.
            continue;
        };
        if name == CAPTURE_LOCK_FILE || name.starts_with(CAPTURE_TEMP_PREFIX) {
            continue;
        }
        if !is_managed_capture_filename(&name) {
            continue;
        }
        let meta = std::fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot stat capture entry {}: {e}", entry.path().display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "symlink entry with a managed capture name is rejected: {}",
                entry.path().display()
            ));
        }
        if !meta.is_file() {
            continue;
        }
        files = files
            .checked_add(1)
            .ok_or_else(|| "capture file count overflow".to_string())?;
        total_bytes = total_bytes
            .checked_add(meta.len())
            .ok_or_else(|| "capture total byte count overflow".to_string())?;
    }
    Ok(RootUsage { files, total_bytes })
}

/// The blocking commit transaction (runs under the process-local mutex in
/// `spawn_blocking`, then under the exclusive advisory root lock).
fn commit_new_file(
    root: &Path,
    name: &str,
    bytes: &[u8],
    limits: CaptureLimits,
) -> Result<CaptureWriteResult, String> {
    validate_capture_filename(name)?;
    // Defense in depth: `write_new` already rejected the per-file quota
    // before the mutex; re-checking here under the lock cannot disagree
    // (limits are Copy) but keeps the invariant local to the transaction.
    let bytes_len = u64::try_from(bytes.len())
        .map_err(|_| "capture file size does not fit a u64 counter".to_string())?;
    if bytes_len > limits.max_file_bytes {
        return Err(format!(
            "capture file exceeds per-file quota: {bytes_len} bytes > --capture-max-file-bytes {}",
            limits.max_file_bytes
        ));
    }

    let _lock_file = acquire_root_lock(root)?;
    let usage = scan_managed_usage(root)?;

    // No-clobber: any existing entry (regular file, symlink, directory,
    // special) at the destination is an error. Checked again atomically by
    // `persist_noclobber` below (the scan is advisory).
    let dest = root.join(name);
    match std::fs::symlink_metadata(&dest) {
        Ok(_) => {
            return Err(format!(
                "capture destination already exists (no overwrite): {}",
                dest.display()
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot stat capture destination {}: {e}",
                dest.display()
            ))
        }
    }

    let new_files = usage
        .files
        .checked_add(1)
        .ok_or_else(|| "capture file count overflow".to_string())?;
    if new_files > limits.max_files {
        return Err(format!(
            "capture file-count quota exceeded: {new_files} files > --capture-max-files {}",
            limits.max_files
        ));
    }
    let new_total = usage
        .total_bytes
        .checked_add(bytes_len)
        .ok_or_else(|| "capture total byte quota overflow".to_string())?;
    if new_total > limits.max_total_bytes {
        return Err(format!(
            "capture total-byte quota exceeded: {new_total} bytes > --capture-max-total-bytes {}",
            limits.max_total_bytes
        ));
    }

    let mut tmp = tempfile::Builder::new()
        .prefix(CAPTURE_TEMP_PREFIX)
        .tempfile_in(root)
        .map_err(|e| format!("cannot create capture temp file in {}: {e}", root.display()))?;
    tmp.write_all(bytes)
        .map_err(|e| format!("cannot write capture temp file: {e}"))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("cannot sync capture temp file: {e}"))?;
    let committed = tmp
        .persist_noclobber(&dest)
        .map_err(|e| format!("capture commit failed: {}", e.error))?;
    drop(committed);
    // Post-commit step: the only fallible operation after the no-clobber
    // persist succeeded. A failure here must NOT turn the commit into an
    // error (the final file already exists and counts toward quota) — it is
    // reported as a durability warning on the successful result.
    let durability_warning = sync_root_dir(root).err();

    Ok(CaptureWriteResult {
        path: dest,
        bytes_written: bytes_len,
        files_used: new_files,
        total_bytes_used: new_total,
        durability_warning,
    })
}

/// Sync the root directory so the rename is durable on crash. Portable
/// std APIs cannot sync a directory on Windows; that crash-durability
/// limitation is documented. Callers must treat an `Err` as a POST-commit
/// durability warning, never as a failed commit.
#[cfg(unix)]
fn sync_root_dir(root: &Path) -> Result<(), String> {
    #[cfg(test)]
    if FAIL_ROOT_SYNC.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("injected root directory sync failure".to_string());
    }
    File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("cannot sync capture dir {}: {e}", root.display()))
}

#[cfg(not(unix))]
fn sync_root_dir(_root: &Path) -> Result<(), String> {
    Ok(())
}

/// Test-only injection point proving the post-commit durability contract:
/// a forced root-sync failure must surface as
/// [`CaptureWriteResult::durability_warning`], never as a failed commit.
#[cfg(test)]
static FAIL_ROOT_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    fn limits() -> CaptureLimits {
        CaptureLimits {
            max_file_bytes: 1024,
            max_total_bytes: 4096,
            max_files: 8,
        }
    }

    #[test]
    fn validate_accepts_portable_names() {
        for name in [
            "boot.log.jsonl",
            "a.jsonl",
            "session-2026-08-01.jsonl",
            "LOG_01.jsonl",
            "x.jsonl",
        ] {
            assert!(
                validate_capture_filename(name).is_ok(),
                "expected {name:?} to be valid"
            );
        }
    }

    #[test]
    fn validate_rejects_traversal_and_paths() {
        for name in [
            "",
            "..",
            ".",
            "a/b.jsonl",
            "a\\b.jsonl",
            "/abs.jsonl",
            "sub/name.jsonl",
            "name.jsonl/",
            "con.jsonl",
            "CON.jsonl",
            "Prn",
            "prn.jsonl",
            "aux.jsonl",
            "NUL.jsonl",
            "com1.jsonl",
            "COM9.jsonl",
            "lpt1.jsonl",
            "LPT9.jsonl",
            "com10.jsonl", // COM10 is NOT reserved (only COM1-9); must be valid
        ] {
            // "com10.jsonl" must be accepted; the rest rejected.
            let expected_ok = name == "com10.jsonl";
            assert_eq!(
                validate_capture_filename(name).is_ok(),
                expected_ok,
                "unexpected verdict for {name:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_bad_shape() {
        for name in [
            "name.txt",
            "name.json",
            "name.JSONL",                    // suffix is case-sensitive
            ".jsonl",                        // starts with '.'
            "-a.jsonl",                      // starts with '-'
            "_a.jsonl",                      // starts with '_'
            "a b.jsonl",                     // space
            "a$b.jsonl",                     // '$'
            "a😀b.jsonl",                    // non-ASCII
            ".serial-mcp-capture-foo.jsonl", // reserved prefix
            "a.serial-mcp-b.jsonl",          // internal reserved prefix
            // Exactly MAX+1 characters (including '.jsonl') is rejected.
            &format!(
                "{}.jsonl",
                "a".repeat(MAX_CAPTURE_FILENAME_LEN + 1 - CAPTURE_FILENAME_SUFFIX.len())
            ), // 115 a's + '.jsonl' = 121 = MAX+1
        ] {
            assert!(
                validate_capture_filename(name).is_err(),
                "expected {name:?} to be rejected"
            );
        }
        // Exactly at the limit is valid.
        let max_len = MAX_CAPTURE_FILENAME_LEN - CAPTURE_FILENAME_SUFFIX.len();
        let at_limit = format!("{}.jsonl", "a".repeat(max_len));
        assert!(validate_capture_filename(&at_limit).is_ok());
        // One character over the limit is rejected (exact boundary).
        let over_limit = format!("{}.jsonl", "a".repeat(max_len + 1));
        assert!(validate_capture_filename(&over_limit).is_err());
    }

    #[test]
    fn open_rejects_invalid_roots() {
        // Relative path.
        assert!(CaptureStore::open("relative".into(), limits()).is_err());
        // Missing directory.
        let missing = tempfile::tempdir().unwrap();
        let missing = missing.path().join("nope");
        assert!(CaptureStore::open(missing, limits()).is_err());
        // Regular file as root.
        let file_dir = tempfile::tempdir().unwrap();
        let file_path = file_dir.path().join("not-a-dir");
        std::fs::write(&file_path, b"x").unwrap();
        assert!(CaptureStore::open(file_path, limits()).is_err());
        // Zero limits and per-file > total.
        let dir = tempfile::tempdir().unwrap();
        let zero = CaptureLimits {
            max_file_bytes: 0,
            ..limits()
        };
        assert!(CaptureStore::open(dir.path().to_path_buf(), zero).is_err());
        let bad_rel = CaptureLimits {
            max_file_bytes: 2048,
            max_total_bytes: 1024,
            max_files: 8,
        };
        assert!(CaptureStore::open(dir.path().to_path_buf(), bad_rel).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn open_rejects_symlink_root() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(CaptureStore::open(link, limits()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn open_rejects_symlink_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim");
        std::fs::write(&target, b"lock").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(CAPTURE_LOCK_FILE)).unwrap();
        let err = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap_err();
        assert!(err.contains("symlink"), "got: {err}");
    }

    #[test]
    fn scan_classifies_managed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), b"12345").unwrap();
        std::fs::write(dir.path().join("b.jsonl"), b"123").unwrap();
        // Non-managed / unknown / internal entries are ignored.
        std::fs::write(dir.path().join("notes.txt"), b"xx").unwrap();
        std::fs::write(dir.path().join("CON.jsonl"), b"xx").unwrap();
        std::fs::create_dir(dir.path().join("d.jsonl")).unwrap();
        std::fs::write(dir.path().join(CAPTURE_LOCK_FILE), b"").unwrap();
        std::fs::write(
            dir.path().join(format!("{CAPTURE_TEMP_PREFIX}abc")),
            b"temp",
        )
        .unwrap();
        let usage = scan_managed_usage(dir.path()).unwrap();
        assert_eq!(usage.files, 2);
        assert_eq!(usage.total_bytes, 8);
    }

    #[cfg(unix)]
    #[test]
    fn scan_rejects_managed_name_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("victim"), b"data").unwrap();
        std::os::unix::fs::symlink(outside.path().join("victim"), dir.path().join("a.jsonl"))
            .unwrap();
        let err = scan_managed_usage(dir.path()).unwrap_err();
        assert!(err.contains("symlink"), "got: {err}");
        // The outside target is untouched.
        assert_eq!(
            std::fs::read(outside.path().join("victim")).unwrap(),
            b"data"
        );
    }

    #[tokio::test]
    async fn write_new_commits_and_reports_usage() {
        let dir = tempfile::tempdir().unwrap();
        let store = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap();
        let res = store
            .write_new("boot.jsonl".into(), b"line1\n".to_vec())
            .await
            .unwrap();
        assert!(res.path.starts_with(dir.path()));
        assert_eq!(res.path.file_name().unwrap(), "boot.jsonl");
        assert_eq!(res.bytes_written, 6);
        assert_eq!(res.files_used, 1);
        assert_eq!(res.total_bytes_used, 6);
        assert_eq!(std::fs::read(&res.path).unwrap(), b"line1\n");
        // No temp debris remains.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(CAPTURE_TEMP_PREFIX)
            })
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn write_new_no_clobber_keeps_original_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let store = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap();
        let first = store
            .write_new("a.jsonl".into(), b"one".to_vec())
            .await
            .unwrap();
        let err = store
            .write_new("a.jsonl".into(), b"two".to_vec())
            .await
            .unwrap_err();
        assert!(
            err.contains("no overwrite") || err.contains("already exists"),
            "got: {err}"
        );
        assert_eq!(std::fs::read(&first.path).unwrap(), b"one");
        assert_eq!(store.max_file_bytes(), 1024);
    }

    #[tokio::test]
    async fn write_new_disabled_errors_before_any_file_work() {
        let store = CaptureStore::disabled();
        let err = store
            .write_new("a.jsonl".into(), b"x".to_vec())
            .await
            .unwrap_err();
        assert_eq!(err, CAPTURE_DISABLED_ERROR);
        assert!(!store.is_enabled());
    }

    #[tokio::test]
    async fn write_new_rejects_invalid_name_and_oversize_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap();
        let err = store
            .write_new("../escape.jsonl".into(), vec![])
            .await
            .unwrap_err();
        assert!(err.contains("separators"), "got: {err}");
        let err = store
            .write_new("big.jsonl".into(), vec![0u8; 2048])
            .await
            .unwrap_err();
        assert!(err.contains("per-file quota"), "got: {err}");
        // Nothing committed.
        assert!(!dir.path().join("big.jsonl").exists());
    }

    #[tokio::test]
    async fn zero_byte_commit_consumes_one_file_slot() {
        let dir = tempfile::tempdir().unwrap();
        let l = CaptureLimits {
            max_file_bytes: 1,
            max_total_bytes: 1, // per-file > 0 required; 0-byte files still fit
            max_files: 1,
        };
        let store = CaptureStore::open(dir.path().to_path_buf(), l).unwrap();
        let res = store.write_new("empty.jsonl".into(), vec![]).await.unwrap();
        assert_eq!(res.bytes_written, 0);
        assert_eq!(res.files_used, 1);
        // Second file now exceeds count quota.
        let err = store
            .write_new("empty2.jsonl".into(), vec![])
            .await
            .unwrap_err();
        assert!(err.contains("file-count quota"), "got: {err}");
    }

    #[tokio::test]
    async fn post_commit_root_sync_failure_is_a_warning_not_a_failed_commit() {
        // Forced post-persist root-sync failure: the file IS committed and
        // counts toward quota; the failure surfaces as durability_warning
        // on the successful result, and the committed file is never deleted.
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let store = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap();
            FAIL_ROOT_SYNC.store(true, std::sync::atomic::Ordering::Relaxed);
            let res = store
                .write_new("sync.jsonl".into(), b"durable?\n".to_vec())
                .await
                .unwrap();
            FAIL_ROOT_SYNC.store(false, std::sync::atomic::Ordering::Relaxed);
            assert!(res.path.is_file(), "the committed file must exist");
            assert_eq!(res.files_used, 1);
            assert_eq!(res.total_bytes_used, 9);
            let warning = res
                .durability_warning
                .expect("root-sync failure must be reported as a warning");
            assert!(warning.contains("sync"), "got: {warning}");
            assert_eq!(std::fs::read(&res.path).unwrap(), b"durable?\n");
            // The lock was released: a follow-up commit still works.
            let res2 = store
                .write_new("after.jsonl".into(), b"ok\n".to_vec())
                .await
                .unwrap();
            assert_eq!(res2.files_used, 2);
            assert!(res2.durability_warning.is_none());
        }
        #[cfg(not(unix))]
        {
            // Windows documents the durability limitation instead; the
            // post-commit step is a no-op, so no warning is ever produced.
            let dir = tempfile::tempdir().unwrap();
            let store = CaptureStore::open(dir.path().to_path_buf(), limits()).unwrap();
            let res = store
                .write_new("sync.jsonl".into(), b"ok\n".to_vec())
                .await
                .unwrap();
            assert!(res.durability_warning.is_none());
        }
    }

    #[tokio::test]
    async fn quotas_persist_across_fresh_store_instances() {
        let dir = tempfile::tempdir().unwrap();
        let l = CaptureLimits {
            max_file_bytes: 64,
            max_total_bytes: 64,
            max_files: 4,
        };
        let s1 = CaptureStore::open(dir.path().to_path_buf(), l).unwrap();
        let r1 = s1
            .write_new("one.jsonl".into(), b"0123456789".repeat(4))
            .await
            .unwrap();
        assert_eq!(r1.total_bytes_used, 40);
        drop(s1);
        // A brand-new store instance scans the same root: quota includes
        // files committed by the previous instance.
        let s2 = CaptureStore::open(dir.path().to_path_buf(), l).unwrap();
        let err = s2
            .write_new("two.jsonl".into(), b"0123456789".repeat(4))
            .await
            .unwrap_err();
        assert!(err.contains("total-byte quota"), "got: {err}");
        let r2 = s2
            .write_new("two.jsonl".into(), b"0123456789".repeat(2))
            .await
            .unwrap();
        assert_eq!(r2.files_used, 2);
        assert_eq!(r2.total_bytes_used, 60);
    }

    #[tokio::test]
    async fn concurrent_independent_stores_cannot_exceed_quota() {
        // Two stores with INDEPENDENT process-local mutexes share one root.
        // Only the cross-process advisory lock serializes them, so count
        // and total quotas hold across processes.
        let dir = StdArc::new(tempfile::tempdir().unwrap());
        let l = CaptureLimits {
            max_file_bytes: 10,
            max_total_bytes: 10,
            max_files: 2,
        };
        let s1 = StdArc::new(CaptureStore::open(dir.path().to_path_buf(), l).unwrap());
        let s2 = StdArc::new(CaptureStore::open(dir.path().to_path_buf(), l).unwrap());
        let barrier = StdArc::new(tokio::sync::Barrier::new(2));

        let t1 = {
            let s1 = StdArc::clone(&s1);
            let b = StdArc::clone(&barrier);
            tokio::spawn(async move {
                b.wait().await;
                s1.write_new("a.jsonl".into(), vec![0u8; 10]).await
            })
        };
        let t2 = {
            let s2 = StdArc::clone(&s2);
            let b = StdArc::clone(&barrier);
            tokio::spawn(async move {
                b.wait().await;
                s2.write_new("b.jsonl".into(), vec![0u8; 10]).await
            })
        };
        let (r1, r2) = tokio::join!(t1, t2);
        let outcomes = [r1.unwrap(), r2.unwrap()];
        let ok: Vec<_> = outcomes.iter().filter(|o| o.is_ok()).collect();
        let err: Vec<_> = outcomes.iter().filter(|o| o.is_err()).collect();
        assert_eq!(ok.len(), 1, "exactly one concurrent commit may succeed");
        assert_eq!(err.len(), 1);
        assert!(
            err[0].as_ref().unwrap_err().contains("quota"),
            "got: {err:?}"
        );
        // The winning commit reports the final usage; the loser changed nothing.
        let usage = scan_managed_usage(dir.path()).unwrap();
        assert_eq!(usage.files, 1);
        assert_eq!(usage.total_bytes, 10);
    }

    #[tokio::test]
    async fn file_count_quota_includes_prior_committed_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits();
        l.max_files = 2;
        let store = CaptureStore::open(dir.path().to_path_buf(), l).unwrap();
        store
            .write_new("one.jsonl".into(), b"1".to_vec())
            .await
            .unwrap();
        store
            .write_new("two.jsonl".into(), b"2".to_vec())
            .await
            .unwrap();
        let err = store
            .write_new("three.jsonl".into(), b"3".to_vec())
            .await
            .unwrap_err();
        assert!(err.contains("file-count quota"), "got: {err}");
        assert!(!dir.path().join("three.jsonl").exists());
    }
}
