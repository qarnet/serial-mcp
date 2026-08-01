//! Write-through profile learning composition (Phase 3B).
//!
//! One shared helper converts a successful live mutation on a
//! profile-bound connection into a revision-CAS persistence attempt and an
//! updated binding, producing the `(ProfileSessionResult,
//! ProfilePersistenceResult)` pair carried by durable tool results.
//!
//! Callers MUST hold the connection's learning lock across the live
//! mutation, this call, and the binding update so concurrent durable
//! requests on one connection cannot snapshot each other's half-applied
//! state.

use std::sync::Arc;

use crate::profile_store::{LearnedUpdate, ProfileStore};
use crate::profiles::{
    ProfilePersistenceOperation, ProfilePersistenceResult, ProfilePersistenceState,
    ProfileSessionResult,
};
use crate::serial::SerialConnection;

/// Classify a store error as a stale-producing conflict: a revision-CAS
/// mismatch or a missing (externally deleted) profile. Plain I/O failures
/// keep the binding dirty but NOT stale so a later durable operation or
/// clean close retries the effective snapshot.
fn is_conflict(err: &str) -> bool {
    err.contains("revision conflict") || err.contains("not found")
}

/// Attempt write-through persistence of `conn`'s effective defaults into
/// its bound profile and update the binding accordingly:
///
/// - no binding or non-persistent binding: `Transient`, no store call
/// - persistent CAS no-op (defaults already equal): `NotNeeded`, binding
///   becomes clean
/// - CAS changed: `Persisted`, binding revision bumped, dirty/stale/error
///   cleared
/// - store failure or conflict after live success: `Failed`, expected
///   revision kept, binding dirty; stale when the error is a conflict or
///   missing profile; the error is recorded
///
/// A stale binding never attempts a store write — it keeps reporting the
/// conflict rather than overwrite a newer/rolled-back profile.
pub async fn learn(
    store: &Arc<ProfileStore>,
    conn: &Arc<SerialConnection>,
    operation: ProfilePersistenceOperation,
) -> (Option<ProfileSessionResult>, ProfilePersistenceResult) {
    let Some(binding) = conn.active_profile_binding() else {
        return (
            None,
            ProfilePersistenceResult {
                state: ProfilePersistenceState::Transient,
                operation,
                profile_name: None,
                revision: None,
                error: None,
            },
        );
    };
    if !binding.persistent {
        return (
            Some(binding.to_session_result()),
            ProfilePersistenceResult {
                state: ProfilePersistenceState::Transient,
                operation,
                profile_name: None,
                revision: None,
                error: None,
            },
        );
    }

    if binding.stale {
        let error = binding.last_persistence_error.clone().unwrap_or_else(|| {
            "profile revision changed externally; connection binding is stale".to_string()
        });
        conn.update_active_profile_binding(|b| {
            b.dirty = true;
            b.last_persistence_error = Some(error.clone());
        });
        return (
            conn.active_profile_binding().map(|b| b.to_session_result()),
            ProfilePersistenceResult {
                state: ProfilePersistenceState::Failed,
                operation,
                profile_name: Some(binding.profile_name),
                revision: binding.revision,
                error: Some(error),
            },
        );
    }

    let expected_revision = binding.revision.unwrap_or(0);
    let defaults = conn.effective_defaults();
    let outcome = store
        .update_learned_defaults(binding.profile_name.clone(), expected_revision, defaults)
        .await;
    match outcome {
        Ok(LearnedUpdate {
            changed: false,
            profile,
        }) => {
            // Durable defaults already equal the effective snapshot: no
            // revision/history bump and no file write. The binding is
            // durably represented, so it becomes clean.
            conn.update_active_profile_binding(|b| {
                b.dirty = false;
                b.stale = false;
                b.last_persistence_error = None;
            });
            (
                conn.active_profile_binding().map(|b| b.to_session_result()),
                ProfilePersistenceResult {
                    state: ProfilePersistenceState::NotNeeded,
                    operation,
                    profile_name: Some(profile.name),
                    revision: Some(profile.metadata.revision),
                    error: None,
                },
            )
        }
        Ok(LearnedUpdate {
            changed: true,
            profile,
        }) => {
            conn.update_active_profile_binding(|b| {
                b.revision = Some(profile.metadata.revision);
                b.dirty = false;
                b.stale = false;
                b.last_persistence_error = None;
            });
            (
                conn.active_profile_binding().map(|b| b.to_session_result()),
                ProfilePersistenceResult {
                    state: ProfilePersistenceState::Persisted,
                    operation,
                    profile_name: Some(profile.name),
                    revision: Some(profile.metadata.revision),
                    error: None,
                },
            )
        }
        Err(e) => {
            let conflict = is_conflict(&e);
            conn.update_active_profile_binding(|b| {
                b.dirty = true;
                if conflict {
                    b.stale = true;
                }
                b.last_persistence_error = Some(e.clone());
            });
            (
                conn.active_profile_binding().map(|b| b.to_session_result()),
                ProfilePersistenceResult {
                    state: ProfilePersistenceState::Failed,
                    operation,
                    profile_name: Some(binding.profile_name),
                    revision: binding.revision,
                    error: Some(e),
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{ProfileDefaults, ProfileMetadata, ProfileSelector};

    #[test]
    fn conflict_classification() {
        assert!(is_conflict(
            "Profile 'dev' revision conflict: expected 1, found 2"
        ));
        assert!(is_conflict("Profile 'dev' not found"));
        assert!(!is_conflict("Cannot create temp file: Permission denied"));
        assert!(!is_conflict("Failed to commit profiles: I/O error"));
    }

    #[test]
    fn learned_update_shape() {
        // The wire types must serialize with snake_case enum names.
        let persisted = ProfilePersistenceResult {
            state: ProfilePersistenceState::Persisted,
            operation: ProfilePersistenceOperation::Learned,
            profile_name: Some("dev".into()),
            revision: Some(2),
            error: None,
        };
        let json = serde_json::to_value(&persisted).unwrap();
        assert_eq!(json["state"], serde_json::json!("persisted"));
        assert_eq!(json["operation"], serde_json::json!("learned"));
        assert_eq!(json["revision"], serde_json::json!(2));

        let failed = ProfilePersistenceResult {
            state: ProfilePersistenceState::Failed,
            operation: ProfilePersistenceOperation::OpenOverride,
            profile_name: Some("dev".into()),
            revision: Some(1),
            error: Some("boom".into()),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["state"], serde_json::json!("failed"));
        assert_eq!(json["operation"], serde_json::json!("open_override"));
        assert_eq!(json["error"], serde_json::json!("boom"));
    }

    #[test]
    fn profile_defaults_partial_eq_detects_snapshot_difference() {
        let a = ProfileDefaults::default();
        let mut b = ProfileDefaults::default();
        assert_eq!(a, b, "identical snapshots are equal");
        b.baud_rate = 9600;
        assert_ne!(a, b, "changed baud must differ");
        let c = ProfileDefaults {
            rx_framing: Some(crate::framing::RxFramingConfig {
                mode: crate::framing::RxFramingMode::Line {
                    ending: crate::framing::LineEnding::Lf,
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_ne!(a, c, "framing must participate in equality");
        let _ = ProfileSelector::default();
        let _ = ProfileMetadata::default();
    }
}
