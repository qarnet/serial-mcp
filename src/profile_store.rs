//! Process-wide persistent profile store.
//!
//! One store is shared by every stdio/HTTP handler:
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
//!   for automatic selection and rollback.
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
    allocate_generated_name, high_identity, normalize_generated_label, rank_candidates,
    selector_matches_high_identity, Profile, ProfileDefaults, ProfileMetadata, ProfileRevision,
    ProfileSelector, MAX_PROFILE_REVISIONS,
};
use crate::serial::PortInfo;

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

/// Outcome of automatic profile resolution for one target port.
#[derive(Debug, Clone, Default)]
pub struct AutomaticResolution {
    /// The uniquely most-recently-used matching profile, when one exists.
    pub selected: Option<Profile>,
    /// `true` when candidates exist but no unique winner (equal top
    /// `last_used_at_ms`) — the caller must use a transient session.
    pub ambiguous: bool,
    /// Names of all candidate profiles that matched the target identity.
    pub candidates: Vec<String>,
}

/// Outcome of a revision-CAS learned update.
#[derive(Debug, Clone)]
pub struct LearnedUpdate {
    /// The effective profile after the attempt (unchanged when `changed`
    /// is false).
    pub profile: Profile,
    /// `true` when defaults changed and the revision/history were bumped
    /// and persisted; `false` for a no-op where durable defaults already
    /// equal the incoming defaults (no file write).
    pub changed: bool,
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

    /// Fresh cross-process read of all profiles for the `list_ports`
    /// preview.
    ///
    /// Persistent stores: acquires the advisory file lock, reloads the file
    /// from disk (never cache-only — another process may have changed
    /// profiles), republishes the cache, and returns the fresh snapshot in
    /// ONE transaction, so a `list_ports` call performs a single lock/reload
    /// regardless of how many ports it previews. Ephemeral stores: returns
    /// the in-memory cache. Corrupt store data is an error (the tool
    /// surfaces it rather than silently claiming no matches).
    pub async fn list_fresh(&self) -> Result<Vec<Profile>, String> {
        self.run_read(|profiles| Ok(profiles.to_vec())).await
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

    /// Fresh automatic resolution for one high-confidence target port.
    ///
    /// Acquires the file lock, reloads the profile file from disk (never
    /// cache-only — another process may have changed profiles), republishes
    /// the cache, and resolves candidates whose selectors carry the target's
    /// high identity fields. Returns the unique most-recently-used profile,
    /// or an ambiguity marker when no unique winner exists.
    ///
    /// Callers must already have verified the target has a high-confidence
    /// identity that is unique among live ports.
    pub async fn resolve_automatic(
        &self,
        target: &PortInfo,
    ) -> Result<AutomaticResolution, String> {
        let identity = high_identity(target).ok_or_else(|| {
            "Automatic resolution requires a high-confidence USB identity".to_string()
        })?;
        let target = target.clone();
        self.run_read(move |profiles| {
            let candidates: Vec<Profile> = profiles
                .iter()
                .filter(|p| {
                    p.matches(&target) && selector_matches_high_identity(&p.selector, &identity)
                })
                .cloned()
                .collect();
            if candidates.is_empty() {
                return Ok(AutomaticResolution::default());
            }
            let names = candidates.iter().map(|p| p.name.clone()).collect();
            if candidates.len() == 1 {
                return Ok(AutomaticResolution {
                    selected: Some(candidates[0].clone()),
                    ambiguous: false,
                    candidates: names,
                });
            }
            let ranked = rank_candidates(candidates);
            let top_ts = ranked[0].metadata.last_used_at_ms.unwrap_or(0);
            let next_ts = ranked[1].metadata.last_used_at_ms.unwrap_or(0);
            if top_ts != next_ts {
                Ok(AutomaticResolution {
                    selected: Some(ranked[0].clone()),
                    ambiguous: false,
                    candidates: names,
                })
            } else {
                Ok(AutomaticResolution {
                    selected: None,
                    ambiguous: true,
                    candidates: names,
                })
            }
        })
        .await
    }

    /// Atomically allocate a generated profile name and create the profile
    /// with generated metadata (revision 1, use count 1, timestamps set).
    /// Never overwrites an existing profile. Returns the effective profile
    /// from the same transaction.
    pub async fn create_generated(
        &self,
        label: String,
        selector: ProfileSelector,
        defaults: ProfileDefaults,
    ) -> Result<Profile, String> {
        self.run_mutation(move |mut profiles, now| {
            let base = format!("auto-{}", normalize_generated_label(&label));
            let name = allocate_generated_name(&profiles, &base);
            let profile = Profile {
                name,
                selector,
                defaults,
                metadata: ProfileMetadata {
                    generated: true,
                    revision: 1,
                    created_at_ms: Some(now),
                    updated_at_ms: Some(now),
                    last_used_at_ms: Some(now),
                    use_count: 1,
                },
                revisions: Vec::new(),
            };
            let effective = profile.clone();
            profiles.push(profile);
            Ok((effective, profiles))
        })
        .await
    }

    /// Mark a profile as used: increment `use_count` and update
    /// `last_used_at_ms`. Does NOT bump the configuration revision or add
    /// history. The timestamp is monotonically greater than any profile's
    /// existing `last_used_at_ms` (`max(now, max_existing + 1)`) so this
    /// server never creates same-millisecond ranking ties. Returns the
    /// effective profile from the same transaction.
    pub async fn mark_used(&self, name: &str) -> Result<Profile, String> {
        let name = name.to_string();
        self.run_mutation(move |mut profiles, now| {
            let idx = profiles
                .iter()
                .position(|p| p.name == name)
                .ok_or_else(|| format!("Profile '{name}' not found"))?;
            let max_existing = profiles
                .iter()
                .filter_map(|p| p.metadata.last_used_at_ms)
                .max()
                .unwrap_or(0);
            let ts = now.max(max_existing.saturating_add(1));
            let old = profiles[idx].clone();
            let updated = Profile {
                metadata: ProfileMetadata {
                    use_count: old.metadata.use_count.saturating_add(1),
                    last_used_at_ms: Some(ts),
                    ..old.metadata.clone()
                },
                ..old
            };
            let effective = updated.clone();
            profiles[idx] = updated;
            Ok((effective, profiles))
        })
        .await
    }

    /// Revision-CAS learned update from a live connection.
    ///
    /// Inside the locked reload-under-lock transaction:
    ///
    /// 1. Requires the profile to exist.
    /// 2. Requires its current metadata revision to equal `expected_revision`.
    /// 3. When `defaults` already equal the current defaults, returns the
    ///    unchanged profile with `changed = false` and does NOT rewrite the
    ///    file, bump the revision, or touch history/timestamps.
    /// 4. Otherwise pushes the current selector/defaults into the bounded
    ///    history, preserves selector + usage/creation/generated metadata,
    ///    bumps the revision, stamps `updated_at_ms`, persists atomically,
    ///    and returns the resulting profile.
    ///
    /// A revision mismatch is an explicit conflict error naming the profile,
    /// expected revision, and actual revision — never a silent last-writer
    /// overwrite.
    pub async fn update_learned_defaults(
        &self,
        profile_name: String,
        expected_revision: u64,
        defaults: ProfileDefaults,
    ) -> Result<LearnedUpdate, String> {
        self.run_conditional_mutation(move |mut profiles, now| {
            let idx = profiles
                .iter()
                .position(|p| p.name == profile_name)
                .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;
            let current = profiles[idx].clone();
            if current.metadata.revision != expected_revision {
                return Err(format!(
                    "Profile '{profile_name}' revision conflict: expected {expected_revision}, \
                     found {}",
                    current.metadata.revision
                ));
            }
            if current.defaults == defaults {
                // No-op: durable defaults already equal the effective
                // snapshot. No revision/history/timestamp bump, and no file
                // rewrite (the mutation returns None).
                return Ok((
                    LearnedUpdate {
                        profile: current,
                        changed: false,
                    },
                    None,
                ));
            }
            let updated = Profile {
                defaults,
                metadata: bump_metadata(&current, now),
                revisions: push_revision(&current, now, current.revisions.clone()),
                ..current
            };
            let effective = updated.clone();
            profiles[idx] = updated;
            Ok((
                LearnedUpdate {
                    profile: effective,
                    changed: true,
                },
                Some(profiles),
            ))
        })
        .await
    }

    /// Roll a profile back to a prior retained revision.
    ///
    /// Requires the current revision to equal `expected_revision`, then:
    ///
    /// 1. finds the requested prior snapshot (a missing/evicted revision is
    ///    a tool error — nothing changes),
    /// 2. pushes the current selector/defaults into the bounded history,
    /// 3. restores the target snapshot's selector/defaults,
    /// 4. sets the new revision to `current + 1` (never backward),
    /// 5. preserves generated/created/last-used/use-count metadata,
    /// 6. stamps `updated_at_ms`, caps history at five, writes atomically,
    /// 7. returns the resulting profile.
    ///
    /// Rollback never touches live hardware; the caller marks affected
    /// same-process bindings stale.
    pub async fn rollback(
        &self,
        profile_name: String,
        expected_revision: u64,
        target_revision: u64,
    ) -> Result<Profile, String> {
        self.run_mutation(move |mut profiles, now| {
            let idx = profiles
                .iter()
                .position(|p| p.name == profile_name)
                .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;
            let current = profiles[idx].clone();
            if current.metadata.revision != expected_revision {
                return Err(format!(
                    "Profile '{profile_name}' revision conflict: expected {expected_revision}, \
                     found {}",
                    current.metadata.revision
                ));
            }
            let retained: Vec<u64> = current.revisions.iter().map(|r| r.revision).collect();
            let snapshot = current
                .revisions
                .iter()
                .find(|r| r.revision == target_revision)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "Profile '{profile_name}' has no retained snapshot at revision \
                         {target_revision} (retained: {})",
                        retained
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            let restored = Profile {
                name: current.name.clone(),
                selector: snapshot.selector.clone(),
                defaults: snapshot.defaults.clone(),
                metadata: ProfileMetadata {
                    generated: current.metadata.generated,
                    revision: current.metadata.revision.saturating_add(1),
                    created_at_ms: current.metadata.created_at_ms,
                    updated_at_ms: Some(now),
                    last_used_at_ms: current.metadata.last_used_at_ms,
                    use_count: current.metadata.use_count,
                },
                revisions: push_revision(&current, now, current.revisions.clone()),
            };
            let effective = restored.clone();
            profiles[idx] = restored;
            Ok((effective, profiles))
        })
        .await
    }

    /// Run one fresh read-only transaction.
    ///
    /// Persistent stores: the compute closure runs inside
    /// [`tokio::task::spawn_blocking`] while holding the process-local
    /// mutation mutex and the advisory file lock, on a fresh read of the
    /// file; the in-memory cache is republished from inside the blocking
    /// transaction so later cache reads see the same state.
    ///
    /// Ephemeral stores: the closure runs against the in-memory cache.
    async fn run_read<T, F>(&self, compute: F) -> Result<T, String>
    where
        F: FnOnce(&[Profile]) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let _guard = self.mutation_lock.lock().await;
        let path = self.path.clone();
        let cache = Arc::clone(&self.cache);
        match path {
            Some(path) => {
                let result = tokio::task::spawn_blocking(move || {
                    let lock = acquire_lock(&path)?;
                    let outcome: Result<T, String> = (|| {
                        let current = load_validated(&path)?;
                        *cache.blocking_write() = current.clone();
                        compute(&current)
                    })();
                    drop(lock);
                    outcome
                })
                .await
                .map_err(|e| format!("Profile store read task failed: {e}"))??;
                Ok(result)
            }
            None => {
                let current = self.cache.read().await.clone();
                compute(&current)
            }
        }
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
        self.run_conditional_mutation(move |profiles, now| {
            apply(profiles, now).map(|(value, next)| (value, Some(next)))
        })
        .await
    }

    /// Like [`Self::run_mutation`], but the apply closure may decide NOT to
    /// write: returning `Ok((value, None))` leaves both the file and the
    /// cache untouched (used by the learned-update no-op path so `NotNeeded`
    /// truly does not rewrite the file).
    ///
    /// On an apply error the cache is republished to the freshly-read disk
    /// state (already in hand under the advisory lock) so a CAS conflict or
    /// external writer becomes visible to later cache reads instead of
    /// hiding behind a stale snapshot.
    async fn run_conditional_mutation<T, F>(&self, apply: F) -> Result<T, String>
    where
        F: FnOnce(Vec<Profile>, u64) -> Result<(T, Option<Vec<Profile>>), String> + Send + 'static,
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
                        let (value, next) = apply(current.clone(), now)?;
                        match next {
                            Some(next) => {
                                write_atomic(&path, &next)?;
                                // Durable write succeeded: publish the full
                                // resulting vector to the shared cache before
                                // releasing the advisory lock.
                                *cache.blocking_write() = next;
                            }
                            None => {
                                // No-op (no file write): the transaction
                                // already loaded the fresh disk state under
                                // the advisory lock, so publish it to the
                                // shared cache — a concurrent external
                                // writer's metadata changes must become
                                // visible to later cache reads without this
                                // transaction rewriting the file.
                                *cache.blocking_write() = current;
                            }
                        }
                        Ok(value)
                    })()
                    .inspect_err(|_e| {
                        // Refresh the cache from the fresh read so conflicts
                        // and external writers are observable.
                        if let Ok(current) = load_validated(&path) {
                            *cache.blocking_write() = current;
                        }
                    });
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
                if let Some(next) = next {
                    *self.cache.write().await = next;
                }
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
/// incoming operation explicitly owns them. Bumps the revision
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

    /// The obsolete `poll_interval_ms` key (removed with the deleted RX
    /// subscription tools) must still load from an existing profile file —
    /// serde ignores the unknown field — and a real durable mutation rewrites
    /// the file without it. No schema-version bump.
    #[tokio::test]
    async fn obsolete_poll_interval_ms_loads_and_is_dropped_on_mutation() {
        let (dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "legacy-poll"
[profile.selector]
vid = 0x1366

[profile.defaults]
baud_rate = 115200
poll_interval_ms = 200
"#,
        )
        .unwrap();

        let store = ProfileStore::open(path.clone()).unwrap();
        let listed = store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "legacy-poll");
        assert_eq!(listed[0].defaults.baud_rate, 115200);
        assert_eq!(listed[0].metadata.revision, 0);

        // A real durable mutation rewrites the file; the obsolete key is
        // absent from the new serialization but the profile survives.
        store
            .update_defaults_preserving_selector(
                "legacy-poll".into(),
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
            !content.contains("poll_interval_ms"),
            "mutation must drop the obsolete key:\n{content}"
        );
        let reloaded = ProfileStore::open(path).unwrap();
        let p = reloaded.get("legacy-poll").await.unwrap();
        assert_eq!(p.defaults.baud_rate, 9600, "settings preserved");
        assert_eq!(p.metadata.revision, 1);
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
        // Generated profiles carry `generated: true`; serialization must not
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

        // Overwrite preserves the generated-profile fields (generated flag,
        // last-used metadata, use count).
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

    // ── Automatic resolution, generated create, mark_used ────────────────

    fn high_usb_port(name: &str, serial: &str, interface: Option<u8>) -> PortInfo {
        PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "Synthetic".into(),
            hardware_id: Some("USB VID:1234 PID:5678".into()),
            transport: crate::serial::PortTransport::Usb,
            vid: Some(0x1234),
            pid: Some(0x5678),
            serial_number: Some(serial.into()),
            manufacturer: Some("Synthetic".into()),
            product: Some("Widget".into()),
            interface,
        }
    }

    #[tokio::test]
    async fn resolve_automatic_fresh_read_and_unique_winner() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let target = high_usb_port("/dev/ttyACM0", "SN-1", None);

        // No candidates yet.
        let resolution = store.resolve_automatic(&target).await.unwrap();
        assert!(resolution.selected.is_none());
        assert!(!resolution.ambiguous);
        assert!(resolution.candidates.is_empty());

        // One matching profile (same high identity, differing path).
        let p = Profile {
            name: "dev-a".into(),
            selector: crate::profiles::canonical_high_selector(&target).unwrap(),
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        store.upsert(p, false).await.unwrap();

        let resolution = store.resolve_automatic(&target).await.unwrap();
        assert!(!resolution.ambiguous);
        assert_eq!(resolution.selected.unwrap().name, "dev-a");
    }

    #[tokio::test]
    async fn resolve_automatic_ignores_weak_selector_profiles() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let target = high_usb_port("/dev/ttyACM0", "SN-1", None);

        // Empty selector profile matches any port via matches(), but must
        // NOT be an automatic candidate for a high-confidence device.
        store
            .upsert(
                Profile {
                    name: "any-device".into(),
                    selector: ProfileSelector::default(),
                    defaults: ProfileDefaults::default(),
                    metadata: ProfileMetadata::default(),
                    revisions: Vec::new(),
                },
                false,
            )
            .await
            .unwrap();
        let resolution = store.resolve_automatic(&target).await.unwrap();
        assert!(resolution.selected.is_none());
        assert!(resolution.candidates.is_empty());
    }

    #[tokio::test]
    async fn resolve_automatic_picks_unique_max_last_used_and_ties_are_ambiguous() {
        // Candidates are prewritten with explicit last-used timestamps
        // (upsert owns metadata on create, so the file is the way to
        // exercise ranking).
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "old"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 100
use_count = 1

[[profile]]
name = "never"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
use_count = 0

[[profile]]
name = "new"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 200
use_count = 3
"#,
        )
        .unwrap();
        let store = ProfileStore::open(path).unwrap();
        let target = high_usb_port("/dev/ttyACM0", "SN-1", None);

        let resolution = store.resolve_automatic(&target).await.unwrap();
        assert!(!resolution.ambiguous);
        assert_eq!(resolution.selected.unwrap().name, "new");
        assert_eq!(resolution.candidates.len(), 3, "all candidates named");

        // Equal top timestamps → ambiguity, candidates named.
        let (_dir2, path2) = temp_path();
        std::fs::write(
            &path2,
            r#"
schema_version = 2

[[profile]]
name = "tie-a"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 300
use_count = 1

[[profile]]
name = "tie-b"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 300
use_count = 1
"#,
        )
        .unwrap();
        let store2 = ProfileStore::open(path2).unwrap();
        let resolution = store2.resolve_automatic(&target).await.unwrap();
        assert!(resolution.ambiguous, "equal top timestamps must tie");
        assert!(resolution.selected.is_none());
        assert_eq!(resolution.candidates, vec!["tie-a", "tie-b"]);

        // Both-None candidates also tie (None sorts oldest → equal rank).
        let (_dir3, path3) = temp_path();
        std::fs::write(
            &path3,
            r#"
schema_version = 2

[[profile]]
name = "never-a"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
use_count = 0

[[profile]]
name = "never-b"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
use_count = 0
"#,
        )
        .unwrap();
        let store3 = ProfileStore::open(path3).unwrap();
        let resolution = store3.resolve_automatic(&target).await.unwrap();
        assert!(resolution.ambiguous, "None == None must tie");
    }

    #[tokio::test]
    async fn resolve_automatic_requires_high_identity() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let weak = PortInfo {
            transport: crate::serial::PortTransport::Unknown,
            ..high_usb_port("/dev/pts/1", "SN-1", None)
        };
        let err = store.resolve_automatic(&weak).await.unwrap_err();
        assert!(
            err.contains("high-confidence"),
            "weak identity must be rejected: {err}"
        );
    }

    #[tokio::test]
    async fn create_generated_allocates_unique_names_and_sets_metadata() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let selector = ProfileSelector {
            vid: Some(0x1234),
            ..Default::default()
        };
        let first = store
            .create_generated(
                "Fake USB Serial".into(),
                selector.clone(),
                ProfileDefaults::default(),
            )
            .await
            .unwrap();
        assert_eq!(first.name, "auto-fake-usb-serial");
        assert!(first.metadata.generated);
        assert_eq!(first.metadata.revision, 1);
        assert_eq!(first.metadata.use_count, 1);
        assert!(first.metadata.created_at_ms.is_some());
        assert!(first.metadata.last_used_at_ms.is_some());

        // Same label → suffix, never overwrite.
        let second = store
            .create_generated(
                "Fake USB Serial".into(),
                selector.clone(),
                ProfileDefaults::default(),
            )
            .await
            .unwrap();
        assert_eq!(second.name, "auto-fake-usb-serial-2");

        // Allocator picks the first free suffix across gaps.
        let third = store
            .create_generated(
                "Fake USB Serial".into(),
                selector,
                ProfileDefaults::default(),
            )
            .await
            .unwrap();
        assert_eq!(third.name, "auto-fake-usb-serial-3");

        let listed = store.list().await;
        assert_eq!(listed.len(), 3);
    }

    #[tokio::test]
    async fn mark_used_bumps_usage_only_and_is_monotonic() {
        // Prewritten metadata (revision 3, use_count 7, history) so the
        // assertions prove mark_used touches ONLY usage fields.
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "dev-a"
[profile.selector]
vid = 0x1234
[profile.metadata]
generated = false
revision = 3
created_at_ms = 100
updated_at_ms = 200
last_used_at_ms = 1000
use_count = 7

[[profile.revisions]]
revision = 2
saved_at_ms = 500
[profile.revisions.selector]
vid = 0x1234
[profile.revisions.defaults]
"#,
        )
        .unwrap();
        let store = ProfileStore::open(path).unwrap();

        let used = store.mark_used("dev-a").await.unwrap();
        assert_eq!(used.metadata.use_count, 8);
        assert_eq!(used.metadata.revision, 3, "revision must not bump");
        assert_eq!(used.metadata.created_at_ms, Some(100));
        assert_eq!(
            used.metadata.updated_at_ms,
            Some(200),
            "usage must not bump updated_at"
        );
        assert!(
            used.metadata.last_used_at_ms.unwrap() > 1000,
            "timestamp must exceed any existing last_used"
        );

        // Second mark_used is strictly greater than the first (no ties).
        let used2 = store.mark_used("dev-a").await.unwrap();
        assert!(
            used2.metadata.last_used_at_ms.unwrap() > used.metadata.last_used_at_ms.unwrap(),
            "mark_used must be monotonic"
        );
        assert_eq!(used2.metadata.use_count, 9);
        assert_eq!(used2.metadata.revision, 3);
        assert_eq!(used2.revisions.len(), 1, "no new history");
        assert_eq!(
            used2.revisions[0].defaults.baud_rate, 115200,
            "history untouched"
        );
    }

    #[tokio::test]
    async fn mark_used_missing_profile_errors() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let err = store.mark_used("no-such").await.unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    // ── Learned CAS updates, no-op detection, rollback ────────────────────

    #[tokio::test]
    async fn update_learned_defaults_bumps_revision_and_preserves_selector_and_generated_metadata()
    {
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "gen-device"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = true
revision = 1
created_at_ms = 100
updated_at_ms = 100
last_used_at_ms = 200
use_count = 4
"#,
        )
        .unwrap();
        let store = ProfileStore::open(path.clone()).unwrap();

        let learned = store
            .update_learned_defaults(
                "gen-device".into(),
                1,
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(learned.changed);
        assert_eq!(learned.profile.defaults.baud_rate, 9600);
        assert_eq!(learned.profile.metadata.revision, 2, "revision bumped");
        assert_eq!(
            learned.profile.selector.vid,
            Some(0x1234),
            "selector preserved"
        );
        assert!(
            learned.profile.metadata.generated,
            "generated flag preserved"
        );
        assert_eq!(learned.profile.metadata.last_used_at_ms, Some(200));
        assert_eq!(learned.profile.metadata.use_count, 4);
        assert_eq!(learned.profile.metadata.created_at_ms, Some(100));
        assert_eq!(
            learned.profile.revisions.len(),
            1,
            "prior state snapshotted"
        );
        assert_eq!(learned.profile.revisions[0].revision, 1);
        assert_eq!(
            learned.profile.revisions[0].defaults.baud_rate, 115200,
            "prior defaults captured"
        );

        let reloaded = ProfileStore::open(path).unwrap();
        let p = reloaded.get("gen-device").await.unwrap();
        assert_eq!(p.metadata.revision, 2, "durable on disk");
        assert_eq!(p.defaults.baud_rate, 9600);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn learned_noop_publishes_fresh_metadata_from_other_writer_without_rewriting_file() {
        use std::os::unix::fs::MetadataExt;
        let (_dir, path) = temp_path();
        // Store A is opened FIRST — its cache predates everything the
        // writer (store B, simulating another process) does on disk.
        let store_a = ProfileStore::open(path.clone()).unwrap();
        let store_b = ProfileStore::open(path.clone()).unwrap();

        // Writer creates the profile and bumps usage metadata on disk.
        store_b
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        store_b.mark_used("dev").await.unwrap();
        let inode_before = std::fs::metadata(&path).unwrap().ino();
        assert!(
            store_a.get("dev").await.is_none(),
            "store A cache is stale: it predates the writer's changes"
        );

        // Store A performs a learned no-op (defaults already equal). The
        // CAS must succeed against the fresh disk revision and the cache
        // must be republished from the fresh read — WITHOUT rewriting the
        // file.
        let learned = store_a
            .update_learned_defaults(
                "dev".into(),
                1,
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!learned.changed, "identical defaults are a no-op");
        assert_eq!(learned.profile.metadata.revision, 1);

        let inode_after = std::fs::metadata(&path).unwrap().ino();
        assert_eq!(
            inode_before, inode_after,
            "no-op must not rewrite the profiles file"
        );

        // The stale store's cache now reflects the writer's fresh
        // metadata (usage bump + last-used timestamp) from the no-op's
        // fresh read.
        let p = store_a.get("dev").await.unwrap();
        assert_eq!(p.metadata.use_count, 1, "fresh usage metadata visible");
        assert!(
            p.metadata.last_used_at_ms.is_some(),
            "fresh last-used metadata visible"
        );
        assert_eq!(p.defaults.baud_rate, 9600);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn update_learned_defaults_noop_returns_changed_false_without_rewriting_file() {
        use std::os::unix::fs::MetadataExt;
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path.clone()).unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        let inode_before = std::fs::metadata(&path).unwrap().ino();
        let before = store.get("dev").await.unwrap();

        // Identical defaults → changed=false; revision/history/timestamps
        // untouched AND the file is not rewritten (rename would change the
        // inode).
        let learned = store
            .update_learned_defaults(
                "dev".into(),
                1,
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!learned.changed);
        assert_eq!(learned.profile.metadata.revision, 1);
        assert_eq!(learned.profile.defaults.baud_rate, 9600);

        let after = store.get("dev").await.unwrap();
        assert_eq!(after.metadata.revision, before.metadata.revision);
        assert_eq!(after.metadata.updated_at_ms, before.metadata.updated_at_ms);
        assert!(after.revisions.is_empty(), "no history on no-op");
        let inode_after = std::fs::metadata(&path).unwrap().ino();
        assert_eq!(
            inode_before, inode_after,
            "no-op must not rewrite the profiles file"
        );
    }

    #[tokio::test]
    async fn update_learned_defaults_conflict_error_names_profile_expected_and_actual() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();

        // Profile is at revision 1; expect revision 7 → explicit conflict
        // naming profile + expected + actual, never a silent overwrite.
        let err = store
            .update_learned_defaults(
                "dev".into(),
                7,
                ProfileDefaults {
                    baud_rate: 19200,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            err.contains("revision conflict")
                && err.contains("'dev'")
                && err.contains("expected 7")
                && err.contains("found 1"),
            "conflict must name profile, expected, and actual revision: {err}"
        );
        // The file must remain untouched by the failed CAS.
        let p = store.get("dev").await.unwrap();
        assert_eq!(p.metadata.revision, 1);
        assert_eq!(p.defaults.baud_rate, 9600);
    }

    #[tokio::test]
    async fn update_learned_defaults_missing_profile_errors() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        let err = store
            .update_learned_defaults("ghost".into(), 1, ProfileDefaults::default())
            .await
            .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn rollback_restores_snapshot_as_new_monotonic_revision() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path).unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 19200,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        // History: [rev1(9600), rev2(19200)] — wait, update_defaults bumps
        // metadata and pushes the prior state; verify the layout first.
        let p = store.get("dev").await.unwrap();
        assert_eq!(p.metadata.revision, 2);
        let retained: Vec<u64> = p.revisions.iter().map(|r| r.revision).collect();
        assert_eq!(retained, vec![1], "one prior snapshot after two writes");
        assert_eq!(p.revisions[0].defaults.baud_rate, 9600);

        // Roll back to revision 1 (9600) with expected revision 2.
        let rolled = store.rollback("dev".into(), 2, 1).await.unwrap();
        assert_eq!(rolled.metadata.revision, 3, "new monotonic revision");
        assert_eq!(rolled.defaults.baud_rate, 9600, "restored defaults");
        assert_eq!(
            rolled.selector,
            ProfileSelector::default(),
            "selector (empty) restored from snapshot"
        );
        // History after rollback: [rev1(9600), rev2(19200)] (current state
        // pushed at rollback time).
        let p = store.get("dev").await.unwrap();
        assert_eq!(p.metadata.revision, 3);
        assert_eq!(p.defaults.baud_rate, 9600);
        let retained: Vec<u64> = p.revisions.iter().map(|r| r.revision).collect();
        assert_eq!(retained, vec![1, 2], "current state pushed into history");
        assert_eq!(p.revisions[1].defaults.baud_rate, 19200);
    }

    #[tokio::test]
    async fn rollback_preserves_generated_and_usage_metadata() {
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"
schema_version = 2

[[profile]]
name = "gen-device"
[profile.selector]
vid = 0x1234
[profile.metadata]
generated = true
revision = 1
created_at_ms = 100
updated_at_ms = 100
last_used_at_ms = 500
use_count = 9
"#,
        )
        .unwrap();
        let store = ProfileStore::open(path.clone()).unwrap();
        store
            .update_learned_defaults(
                "gen-device".into(),
                1,
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let rolled = store.rollback("gen-device".into(), 2, 1).await.unwrap();
        assert_eq!(rolled.metadata.revision, 3);
        assert!(rolled.metadata.generated, "generated preserved");
        assert_eq!(rolled.metadata.last_used_at_ms, Some(500));
        assert_eq!(rolled.metadata.use_count, 9);
        assert_eq!(rolled.metadata.created_at_ms, Some(100));

        let reloaded = ProfileStore::open(path).unwrap();
        let p = reloaded.get("gen-device").await.unwrap();
        assert!(p.metadata.generated, "durable on disk");
        assert_eq!(p.metadata.revision, 3);
    }

    #[tokio::test]
    async fn rollback_wrong_expected_revision_and_evicted_revision_error_without_change() {
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path.clone()).unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 9600,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        store
            .update_defaults_preserving_selector(
                "dev".into(),
                ProfileDefaults {
                    baud_rate: 19200,
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();

        // Wrong expected revision → conflict, file unchanged.
        let err = store.rollback("dev".into(), 1, 1).await.unwrap_err();
        assert!(
            err.contains("revision conflict")
                && err.contains("expected 1")
                && err.contains("found 2"),
            "got: {err}"
        );
        let p = store.get("dev").await.unwrap();
        assert_eq!(p.metadata.revision, 2, "file unchanged after wrong CAS");

        // Evicted/missing target revision → tool error, file unchanged.
        let err = store.rollback("dev".into(), 2, 99).await.unwrap_err();
        assert!(
            err.contains("no retained snapshot at revision 99"),
            "got: {err}"
        );
        let p = store.get("dev").await.unwrap();
        assert_eq!(
            p.metadata.revision, 2,
            "file unchanged after evicted rollback"
        );
        assert_eq!(p.defaults.baud_rate, 19200);

        // Missing profile → error.
        let err = store.rollback("ghost".into(), 1, 1).await.unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn concurrent_generated_allocations_never_overwrite() {
        // Two concurrent create_generated calls on the same store must
        // produce distinct names (serialized by the mutation lock + file
        // lock) and persist both.
        let (_dir, path) = temp_path();
        let store = Arc::new(ProfileStore::open(path).unwrap());
        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);
        let (r1, r2) = tokio::join!(
            s1.create_generated(
                "Widget".into(),
                ProfileSelector::default(),
                ProfileDefaults::default()
            ),
            s2.create_generated(
                "Widget".into(),
                ProfileSelector::default(),
                ProfileDefaults::default()
            ),
        );
        let (p1, p2) = (r1.unwrap(), r2.unwrap());
        assert_ne!(p1.name, p2.name, "concurrent allocation must not collide");

        let listed = store.list().await;
        assert_eq!(listed.len(), 2);
        let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&p1.name.as_str()));
        assert!(names.contains(&p2.name.as_str()));
    }

    #[tokio::test]
    async fn resolve_automatic_observes_other_writers_via_fresh_read() {
        // The fresh-read path must see profiles written by a DIFFERENT
        // store instance (simulating another process), not a stale cache.
        let (_dir, path) = temp_path();
        let store = ProfileStore::open(path.clone()).unwrap();
        let target = high_usb_port("/dev/ttyACM0", "SN-1", None);

        let other = ProfileStore::open(path.clone()).unwrap();
        other
            .upsert(
                Profile {
                    name: "from-other-process".into(),
                    selector: crate::profiles::canonical_high_selector(&target).unwrap(),
                    defaults: ProfileDefaults::default(),
                    metadata: ProfileMetadata::default(),
                    revisions: Vec::new(),
                },
                false,
            )
            .await
            .unwrap();

        let resolution = store.resolve_automatic(&target).await.unwrap();
        assert_eq!(
            resolution.selected.unwrap().name,
            "from-other-process",
            "resolve must read the file fresh, not a stale cache"
        );
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
