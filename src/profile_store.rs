//! Process-wide persistent profile store.
//!
//! Replaces the handler-local `Vec<Profile>` + path with a single store
//! shared by every stdio/HTTP handler:
//!
//! - loads and validates the profile file on the production startup path,
//! - serializes all in-process mutations behind one async mutex,
//! - serializes cross-process mutations with an advisory lock file
//!   (`<profiles file>.lock`) and reloads the file under that lock so two
//!   server processes cannot lose each other's updates,
//! - persists atomically (temp file + `sync_all` + rename),
//! - migrates legacy unversioned TOML (schema v1) to the current v2 format
//!   in memory and rejects corrupt or unsupported-future files instead of
//!   silently treating them as empty,
//! - maintains per-profile metadata and a bounded prior-revision history
//!   for the Phase 3 automatic-selection/rollback work.
//!
//! Ephemeral stores (`ProfileStore::ephemeral`) keep the same in-memory
//! semantics for tests/library construction but never touch disk.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::profiles::{
    Profile, ProfileDefaults, ProfileMetadata, ProfileRevision, ProfileSelector,
    MAX_PROFILE_REVISIONS,
};

/// Current on-disk schema version. Version 1 was the legacy unversioned
/// format; version 2 adds `schema_version`, `ProfileMetadata`, and
/// `ProfileRevision` records.
const CURRENT_SCHEMA_VERSION: u32 = 2;

/// The schema version assumed when the file predates `schema_version`.
const LEGACY_SCHEMA_VERSION: u32 = 1;

/// TOML root structure for the profiles file.
#[derive(Debug, Deserialize, Serialize)]
struct ProfilesFile {
    #[serde(default = "legacy_schema_version")]
    schema_version: u32,
    #[serde(default)]
    profile: Vec<Profile>,
}

fn legacy_schema_version() -> u32 {
    LEGACY_SCHEMA_VERSION
}

/// Process-wide store of named profiles.
///
/// `path == None` marks an ephemeral store: mutations only touch the
/// in-memory cache. `path == Some(p)` marks a persistent store: every
/// mutation reloads the file under an advisory lock and atomically
/// replaces it before the in-memory cache is updated.
#[derive(Debug)]
pub struct ProfileStore {
    path: Option<PathBuf>,
    cache: Arc<RwLock<Vec<Profile>>>,
    mutation_lock: Mutex<()>,
}

impl ProfileStore {
    /// Open a persistent store at `path`. A missing file is a valid empty
    /// current-version store (directories are created on first mutation).
    /// A corrupt file, `schema_version == 0`, or an unsupported future
    /// version returns an error so callers can fail startup clearly.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let profiles = load_validated(&path)?;
        Ok(Self {
            path: Some(path),
            cache: Arc::new(RwLock::new(profiles)),
            mutation_lock: Mutex::new(()),
        })
    }

    /// Open an ephemeral store that never touches disk.
    pub fn ephemeral() -> Self {
        Self {
            path: None,
            cache: Arc::new(RwLock::new(Vec::new())),
            mutation_lock: Mutex::new(()),
        }
    }

    /// The backing file path, if this is a persistent store.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Snapshot of all profiles in memory. Read-only; never holds a
    /// mutation lock.
    pub async fn list(&self) -> Vec<Profile> {
        self.cache.read().await.clone()
    }

    /// Look up a single profile by name.
    pub async fn get(&self, name: &str) -> Option<Profile> {
        self.cache
            .read()
            .await
            .iter()
            .find(|p| p.name == name)
            .cloned()
    }

    /// Insert or replace `profile`.
    ///
    /// Returns `true` when the profile was newly created, `false` when it
    /// replaced an existing profile of the same name. Without `overwrite`,
    /// an existing name is a duplicate error.
    pub async fn upsert(&self, profile: Profile, overwrite: bool) -> Result<bool, String> {
        self.run_mutation(move |profiles, now| apply_upsert(profiles, profile, overwrite, now))
            .await
    }

    /// Update a profile's defaults while preserving its selector.
    ///
    /// Profile mode of the `configure` tool. Creates the profile (with an
    /// empty selector) when it does not exist; replaces the defaults of an
    /// existing profile only with `overwrite`. Returns the creation flag
    /// together with the effective resulting profile (freshly read under
    /// the lock), so the caller never needs a racy second lookup.
    pub async fn update_defaults_preserving_selector(
        &self,
        name: String,
        defaults: ProfileDefaults,
        overwrite: bool,
    ) -> Result<(bool, Profile), String> {
        self.run_mutation(move |profiles, now| {
            apply_update_defaults(profiles, name, defaults, overwrite, now)
        })
        .await
    }

    /// Delete a profile by name. Errors when no such profile exists.
    pub async fn delete(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        self.run_mutation(move |profiles, _now| apply_delete(profiles, name))
            .await
    }

    /// Run one read-modify-write mutation.
    ///
    /// Persistent stores: the apply closure runs inside
    /// [`tokio::task::spawn_blocking`] while holding the process-local
    /// mutation mutex and the advisory file lock, on a fresh read of the
    /// file. The in-memory cache is replaced from inside the blocking
    /// transaction — immediately after the durable write succeeds and
    /// before the advisory lock is released — so a cancelled/dropped
    /// awaiting task cannot leave disk updated but cache stale: the
    /// blocking task finishes the cache publication regardless, and any
    /// later mutation is serialized behind the advisory lock. Any failure
    /// leaves both the previous file and the cache unchanged.
    ///
    /// Ephemeral stores: the same closure runs against the in-memory
    /// cache, serialized by the same mutation mutex.
    async fn run_mutation<T, F>(&self, apply: F) -> Result<T, String>
    where
        F: FnOnce(Vec<Profile>, u64) -> Result<(T, Vec<Profile>), String> + Send + 'static,
        T: Send + 'static,
    {
        let _guard = self.mutation_lock.lock().await;
        let now = now_ms();
        let path = self.path.clone();
        let cache = Arc::clone(&self.cache);
        match path {
            Some(path) => {
                let result = tokio::task::spawn_blocking(move || {
                    let lock = acquire_lock(&path)?;
                    let outcome: Result<T, String> = (|| {
                        let current = load_validated(&path)?;
                        let (value, next) = apply(current, now)?;
                        write_atomic(&path, &next)?;
                        // Durable write succeeded: publish the full
                        // resulting vector to the shared cache before
                        // releasing the advisory lock.
                        *cache.blocking_write() = next;
                        Ok(value)
                    })();
                    // Dropping the lock file releases the advisory lock.
                    drop(lock);
                    outcome
                })
                .await
                .map_err(|e| format!("Profile store write task failed: {e}"))??;
                Ok(result)
            }
            None => {
                let current = self.cache.read().await.clone();
                let (value, next) = apply(current, now)?;
                *self.cache.write().await = next;
                Ok(value)
            }
        }
    }
}

// ---- Read / write / lock primitives (blocking) ----------------------------

/// Read and validate the profile file. A missing file is an empty
/// current-version store. Legacy unversioned files parse as v1 and migrate
/// in memory (missing metadata/revision fields get their defaults). Any
/// parse error, `schema_version == 0`, or version newer than the current
/// one is an error — never an empty store.
fn load_validated(path: &Path) -> Result<Vec<Profile>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Cannot read profiles file {}: {e}", path.display())),
    };

    let file: ProfilesFile = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse profiles file {}: {e}", path.display()))?;

    if file.schema_version == 0 {
        return Err(format!(
            "Profiles file {} declares schema_version 0, which is invalid",
            path.display()
        ));
    }
    if file.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Profiles file {} uses schema version {}, which is newer than this \
             build supports (max {CURRENT_SCHEMA_VERSION}). Upgrade serial-mcp \
             or fix the file; it was left unchanged.",
            path.display(),
            file.schema_version
        ));
    }

    Ok(file.profile)
}

/// The sibling lock file: `<profiles file path>.lock`.
fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// Parent directory of a path, falling back to `.` so a bare relative
/// filename (`--profiles-path profiles.toml`) resolves against the current
/// working directory.
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Create the parent directory (if needed) and open the sibling lock file
/// with an exclusive advisory lock. Blocks while another process holds the
/// lock; callers must run this off the async runtime.
fn acquire_lock(path: &Path) -> Result<File, String> {
    let dir = parent_dir(path);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create profile dir: {e}"))?;

    let lock = lock_path(path);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock)
        .map_err(|e| format!("Cannot open lock file {}: {e}", lock.display()))?;
    file.lock_exclusive()
        .map_err(|e| format!("Cannot lock profiles file {}: {e}", lock.display()))?;
    Ok(file)
}

/// Serialize the current schema and atomically replace `path` with a
/// `NamedTempFile` that was written and `sync_all()`-ed before the rename.
fn write_atomic(path: &Path, profiles: &[Profile]) -> Result<(), String> {
    let file = ProfilesFile {
        schema_version: CURRENT_SCHEMA_VERSION,
        profile: profiles.to_vec(),
    };
    let toml =
        toml::to_string_pretty(&file).map_err(|e| format!("Failed to serialize profiles: {e}"))?;

    let dir = parent_dir(path);
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    tmp.write_all(toml.as_bytes())
        .map_err(|e| format!("Failed to write profiles: {e}"))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("Failed to sync profiles: {e}"))?;
    tmp.persist(path)
        .map_err(|e| format!("Failed to commit profiles: {e}"))?;
    Ok(())
}

// ---- Pure read-modify-write operations ------------------------------------

fn create_metadata(now: u64) -> ProfileMetadata {
    ProfileMetadata {
        generated: false,
        revision: 1,
        created_at_ms: Some(now),
        updated_at_ms: Some(now),
        last_used_at_ms: None,
        use_count: 0,
    }
}

/// Next metadata for an overwrite: preserve the original creation
/// timestamp, generated flag, last-used metadata, and use count unless the
/// incoming operation explicitly owns them (Phase 3). Bumps the revision
/// (a legacy/default revision 0 becomes 1 on the first update) and stamps
/// `updated_at_ms`.
fn bump_metadata(old: &Profile, now: u64) -> ProfileMetadata {
    ProfileMetadata {
        generated: old.metadata.generated,
        revision: old.metadata.revision.saturating_add(1),
        created_at_ms: old.metadata.created_at_ms,
        updated_at_ms: Some(now),
        last_used_at_ms: old.metadata.last_used_at_ms,
        use_count: old.metadata.use_count,
    }
}

/// Append a snapshot of `old`'s selector/defaults to its revision history,
/// keeping only the newest [`MAX_PROFILE_REVISIONS`] snapshots. The
/// snapshot carries the revision number that owned the prior state.
fn push_revision(
    old: &Profile,
    now: u64,
    mut revisions: Vec<ProfileRevision>,
) -> Vec<ProfileRevision> {
    revisions.push(ProfileRevision {
        revision: old.metadata.revision,
        saved_at_ms: now,
        selector: old.selector.clone(),
        defaults: old.defaults.clone(),
    });
    let excess = revisions.len().saturating_sub(MAX_PROFILE_REVISIONS);
    if excess > 0 {
        revisions.drain(..excess);
    }
    revisions
}

/// `save_profile` store operation.
fn apply_upsert(
    mut profiles: Vec<Profile>,
    incoming: Profile,
    overwrite: bool,
    now: u64,
) -> Result<(bool, Vec<Profile>), String> {
    let existing_idx = profiles.iter().position(|p| p.name == incoming.name);
    if existing_idx.is_some() && !overwrite {
        return Err(format!(
            "Profile '{}' already exists. Set overwrite=true to replace.",
            incoming.name
        ));
    }

    match existing_idx {
        Some(idx) => {
            let old = profiles[idx].clone();
            profiles[idx] = Profile {
                name: incoming.name,
                selector: incoming.selector,
                defaults: incoming.defaults,
                metadata: bump_metadata(&old, now),
                revisions: push_revision(&old, now, old.revisions.clone()),
            };
            Ok((false, profiles))
        }
        None => {
            profiles.push(Profile {
                name: incoming.name,
                selector: incoming.selector,
                defaults: incoming.defaults,
                metadata: create_metadata(now),
                revisions: Vec::new(),
            });
            Ok((true, profiles))
        }
    }
}

/// `configure(profile=...)` store operation: replace defaults, preserve
/// the selector (read fresh from the file under the lock). Returns the
/// creation flag plus the effective resulting profile so callers can
/// build tool results without a racy second lookup.
fn apply_update_defaults(
    mut profiles: Vec<Profile>,
    name: String,
    defaults: ProfileDefaults,
    overwrite: bool,
    now: u64,
) -> Result<((bool, Profile), Vec<Profile>), String> {
    let existing_idx = profiles.iter().position(|p| p.name == name);
    if existing_idx.is_some() && !overwrite {
        return Err(format!(
            "Profile '{name}' already exists. Set overwrite=true to replace."
        ));
    }

    match existing_idx {
        Some(idx) => {
            let old = profiles[idx].clone();
            let merged = Profile {
                name: name.clone(),
                selector: old.selector.clone(),
                defaults,
                metadata: bump_metadata(&old, now),
                revisions: push_revision(&old, now, old.revisions.clone()),
            };
            profiles[idx] = merged.clone();
            Ok(((false, merged), profiles))
        }
        None => {
            let created = Profile {
                name: name.clone(),
                selector: ProfileSelector::default(),
                defaults,
                metadata: create_metadata(now),
                revisions: Vec::new(),
            };
            profiles.push(created.clone());
            Ok(((true, created), profiles))
        }
    }
}

/// `delete_profile` store operation.
fn apply_delete(mut profiles: Vec<Profile>, name: String) -> Result<((), Vec<Profile>), String> {
    let len_before = profiles.len();
    profiles.retain(|p| p.name != name);
    if profiles.len() == len_before {
        return Err(format!("Profile '{name}' not found"));
    }
    Ok(((), profiles))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Snapshot a `Profile` used by tests.
#[cfg(test)]
fn test_profile(name: &str) -> Profile {
    Profile {
        name: name.into(),
        selector: ProfileSelector {
            vid: Some(0x1234),
            ..Default::default()
        },
        defaults: ProfileDefaults::default(),
        metadata: ProfileMetadata::default(),
        revisions: Vec::new(),
    }
}

/// Test-only clone of a `ProfileStore` shareable across tasks (the store
/// itself is not `Clone`).
#[cfg(test)]
impl ProfileStore {
    fn clone_for_test(&self) -> Arc<ProfileStore> {
        Arc::new(ProfileStore {
            path: self.path.clone(),
            cache: Arc::clone(&self.cache),
            mutation_lock: Mutex::new(()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        (dir, path)
    }

    #[tokio::test]
    async fn open_missing_file_is_empty_store() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        assert!(store.list().await.is_empty());
        assert!(store.path().is_some());
    }

    #[test]
    fn open_invalid_toml_errors() {
        let (dir, path) = temp_path();
        std::fs::write(&path, "not valid toml {{{").unwrap();
        let err = ProfileStore::open(path).unwrap_err();
        assert!(err.contains("Failed to parse"), "got: {err}");
        drop(dir);
    }

    #[test]
    fn open_future_schema_version_errors() {
        let (dir, path) = temp_path();
        std::fs::write(&path, "schema_version = 999\n").unwrap();
        let err = ProfileStore::open(path).unwrap_err();
        assert!(err.contains("newer than this build supports"), "got: {err}");
        drop(dir);
    }

    #[test]
    fn open_zero_schema_version_errors() {
        let (dir, path) = temp_path();
        std::fs::write(&path, "schema_version = 0\n").unwrap();
        let err = ProfileStore::open(path).unwrap_err();
        assert!(err.contains("schema_version 0"), "got: {err}");
        drop(dir);
    }

    #[tokio::test]
    async fn legacy_unversioned_file_loads_and_migrates_on_mutation() {
        let (dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
[[profile]]
name = "nrf-dk"
[profile.selector]
vid = 0x1366
[profile.defaults]
baud_rate = 115200
"#,
        )
        .unwrap();

        let store = ProfileStore::open(path.clone()).unwrap();
        let listed = store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "nrf-dk");
        assert_eq!(listed[0].selector.vid, Some(0x1366));
        assert_eq!(listed[0].metadata.revision, 0, "legacy metadata defaults");

        // First mutation writes the current schema version and bumps the
        // legacy profile to revision 1, preserving settings.
        store
            .update_defaults_preserving_selector(
                "nrf-dk".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("schema_version = 2"),
            "mutation must write current schema version:\n{content}"
        );
        let reloaded = ProfileStore::open(path).unwrap();
        let p = reloaded.get("nrf-dk").await.unwrap();
        assert_eq!(p.defaults.baud_rate, 9600, "settings preserved");
        assert_eq!(p.selector.vid, Some(0x1366), "selector preserved");
        assert_eq!(p.metadata.revision, 1, "legacy revision becomes 1");
        assert_eq!(p.revisions.len(), 1, "prior state snapshotted");
        drop(dir);
    }

    #[tokio::test]
    async fn upsert_creates_new_profile_with_metadata() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let created = store.upsert(test_profile("dev-a"), false).await.unwrap();
        assert!(created);

        let p = store.get("dev-a").await.unwrap();
        assert_eq!(p.metadata.revision, 1);
        assert!(!p.metadata.generated);
        assert!(p.metadata.created_at_ms.is_some());
        assert!(p.metadata.updated_at_ms.is_some());
        assert_eq!(p.metadata.last_used_at_ms, None);
        assert_eq!(p.metadata.use_count, 0);
        assert!(p.revisions.is_empty());
    }

    #[tokio::test]
    async fn upsert_duplicate_rejected_without_overwrite() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store.upsert(test_profile("dev-a"), false).await.unwrap();
        let err = store
            .upsert(test_profile("dev-a"), false)
            .await
            .unwrap_err();
        assert!(
            err.contains("already exists") && err.contains("overwrite=true"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn upsert_overwrite_preserves_metadata_and_snapshots_prior_state() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store.upsert(test_profile("dev-a"), false).await.unwrap();
        let original_created_at = store.get("dev-a").await.unwrap().metadata.created_at_ms;

        let mut updated = test_profile("dev-a");
        updated.defaults.baud_rate = 9600;
        updated.selector.vid = Some(0xAAAA);
        let created = store.upsert(updated, true).await.unwrap();
        assert!(!created);

        let p = store.get("dev-a").await.unwrap();
        assert_eq!(
            p.metadata.created_at_ms, original_created_at,
            "created_at preserved"
        );
        assert_eq!(p.metadata.revision, 2, "revision incremented");
        assert_eq!(p.defaults.baud_rate, 9600, "new defaults applied");
        assert_eq!(p.selector.vid, Some(0xAAAA), "new selector applied");
        assert_eq!(p.revisions.len(), 1, "prior state snapshotted");
        assert_eq!(p.revisions[0].revision, 1, "snapshot owns prior revision");
        assert_eq!(
            p.revisions[0].defaults.baud_rate, 115200,
            "snapshot has prior defaults"
        );
        assert_eq!(
            p.revisions[0].selector.vid,
            Some(0x1234),
            "snapshot has prior selector"
        );
    }

    #[tokio::test]
    async fn revision_history_capped_at_five() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store.upsert(test_profile("dev-a"), false).await.unwrap();
        for i in 0..7 {
            let mut updated = test_profile("dev-a");
            updated.defaults.baud_rate = 1000 + i;
            store.upsert(updated, true).await.unwrap();
        }
        let p = store.get("dev-a").await.unwrap();
        assert_eq!(p.revisions.len(), MAX_PROFILE_REVISIONS);
        assert_eq!(p.metadata.revision, 8, "seven overwrites after create");
        // Newest snapshot is the newest prior state.
        assert_eq!(p.revisions[4].defaults.baud_rate, 1005);
        // Oldest retained snapshot is the fifth-newest prior state.
        assert_eq!(p.revisions[0].defaults.baud_rate, 1001);
    }

    #[tokio::test]
    async fn update_defaults_preserving_selector_keeps_disk_selector() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store.upsert(test_profile("dev-a"), false).await.unwrap();

        // configure() with overwrite replaces defaults but must keep the
        // selector that was written to disk, and must return the effective
        // profile atomically (no racy second lookup needed).
        let (created, effective) = store
            .update_defaults_preserving_selector(
                "dev-a".into(),
                ProfileDefaults {
                    baud_rate: 19200,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        assert!(!created);
        assert_eq!(effective.selector.vid, Some(0x1234), "selector preserved");
        assert_eq!(effective.defaults.baud_rate, 19200, "defaults replaced");
        // The store's own view agrees with the returned profile.
        let p = store.get("dev-a").await.unwrap();
        assert_eq!(p.defaults.baud_rate, effective.defaults.baud_rate);
        assert_eq!(p.selector.vid, effective.selector.vid);
    }

    #[tokio::test]
    async fn update_defaults_creates_with_empty_selector() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let (created, p) = store
            .update_defaults_preserving_selector(
                "new-pro".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                false,
            )
            .await
            .unwrap();
        assert!(created);
        assert_eq!(p.selector, ProfileSelector::default());
        assert_eq!(p.defaults.baud_rate, 9600);
        assert_eq!(p.metadata.revision, 1);
    }

    #[tokio::test]
    async fn delete_removes_and_missing_errors() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store.upsert(test_profile("dev-a"), false).await.unwrap();
        store.upsert(test_profile("dev-b"), false).await.unwrap();

        store.delete("dev-a").await.unwrap();
        let listed = store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "dev-b");

        let err = store.delete("no-such").await.unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn persistent_mutations_survive_store_reopen() {
        let (_dir, path) = temp_path();
        {
            let store = ProfileStore::open(path.clone()).unwrap();
            store.upsert(test_profile("dev-a"), false).await.unwrap();
            store.upsert(test_profile("dev-b"), false).await.unwrap();
        }
        let reopened = ProfileStore::open(path).unwrap();
        let names: Vec<String> = reopened.list().await.into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["dev-a", "dev-b"]);
    }

    #[tokio::test]
    async fn ephemeral_store_mutates_in_memory_only() {
        let store = ProfileStore::ephemeral();
        assert!(store.path().is_none());
        store.upsert(test_profile("dev-a"), false).await.unwrap();
        let listed = store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "dev-a");
    }

    #[tokio::test]
    async fn generated_metadata_round_trips_through_file() {
        // Phase 3 profiles carry `generated: true`; serialization must not
        // depend on the profile name and must survive a reopen. A prewritten
        // v2 file's metadata (including an overwrite preserving it) must
        // come back intact.
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "gen-device"
[profile.metadata]
generated = true
revision = 7
created_at_ms = 1000
updated_at_ms = 2000
last_used_at_ms = 3000
use_count = 3
"#,
        )
        .unwrap();

        let store = ProfileStore::open(path.clone()).unwrap();
        let p = store.get("gen-device").await.unwrap();
        assert!(p.metadata.generated, "generated flag preserved");
        assert_eq!(p.metadata.last_used_at_ms, Some(3000));
        assert_eq!(p.metadata.use_count, 3);
        assert_eq!(p.metadata.revision, 7);

        // Overwrite preserves the Phase 3-owned fields (generated flag,
        // last-used metadata, use count) per the handoff.
        let mut updated = test_profile("gen-device");
        updated.defaults.baud_rate = 9600;
        store.upsert(updated, true).await.unwrap();
        let p = store.get("gen-device").await.unwrap();
        assert!(
            p.metadata.generated,
            "generated flag preserved on overwrite"
        );
        assert_eq!(p.metadata.last_used_at_ms, Some(3000));
        assert_eq!(p.metadata.use_count, 3);
        assert_eq!(p.metadata.created_at_ms, Some(1000));
        assert_eq!(p.metadata.revision, 8);

        let reloaded = ProfileStore::open(path).unwrap();
        let p = reloaded.get("gen-device").await.unwrap();
        assert!(p.metadata.generated, "generated flag survives reopen");
        assert_eq!(p.metadata.revision, 8);
    }

    #[test]
    fn parent_dir_resolves_bare_relative_names_to_cwd() {
        // A bare relative `--profiles-path profiles.toml` must create files
        // in the current working directory, not choke on an empty parent.
        assert_eq!(parent_dir(Path::new("profiles.toml")), PathBuf::from("."));
        assert_eq!(
            parent_dir(Path::new("sub/dir/profiles.toml")),
            PathBuf::from("sub/dir")
        );
        assert_eq!(
            parent_dir(Path::new("/abs/profiles.toml")),
            PathBuf::from("/abs")
        );
        // End-to-end relative behavior (real process cwd) is covered by the
        // http_integration `relative_profiles_path` test.
    }

    #[tokio::test]
    async fn cancelled_upsert_still_publishes_cache_and_disk() {
        // Regression: a persistent mutation whose awaiting task is dropped
        // (tool cancellation) must still complete its disk write AND its
        // cache publication, because both happen inside the blocking
        // transaction before the advisory lock is released.
        let (_dir, path) = temp_path();
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path(&path))
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let store = ProfileStore::open(path.clone()).unwrap();
        let store_for_task = store.clone_for_test();
        let task = tokio::spawn(async move {
            let _ = store_for_task
                .upsert(test_profile("cancelled-dev"), false)
                .await;
        });

        // Give the task time to reach the blocking transaction, which is
        // now parked on our exclusive lock.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        task.abort();
        let _ = task.await; // join handle drops the aborted future

        // Nothing visible yet: the transaction is still blocked.
        assert!(store.get("cancelled-dev").await.is_none());

        // Release the lock; the abandoned blocking task now completes the
        // durable write and the cache publication by itself.
        FileExt::unlock(&lock_file).unwrap();
        drop(lock_file);

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if store.get("cancelled-dev").await.is_some() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "mutation must appear in the live store even after the awaiting task was aborted"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // And it must be durably on disk for a fresh store.
        let reloaded = ProfileStore::open(path).unwrap();
        assert!(
            reloaded.get("cancelled-dev").await.is_some(),
            "aborted mutation must also appear in the reopened disk state"
        );
    }
}
