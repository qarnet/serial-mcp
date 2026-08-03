//! Product-owned MCP protocol version policy.
//!
//! Single production source for the advertised protocol versions and their
//! exact lifecycle / capability / cache contracts. A version is supported
//! ONLY when it has a row in [`SUPPORTED_PROTOCOLS`] — never because rmcp
//! knows it or because its date sorts after a known version. Adding or
//! changing support is a one-row policy edit plus a test row.

use rmcp::model::ProtocolVersion;

/// Lifecycle contract for a protocol version: how a peer enters the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProtocolLifecycle {
    /// `initialize` + session state (`2025-11-25` legacy lifecycle).
    InitializeSession,
    /// `server/discover` + stateless requests (`2026-07-28` modern lifecycle).
    DiscoverStateless,
}

/// SEP-2549 cache-field policy for a protocol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CachePolicy {
    /// No `ttlMs`/`cacheScope` fields. Legacy peers must never see them;
    /// rmcp strips `resultType` for legacy but does NOT strip cache fields,
    /// so the server omits them itself.
    Omit,
    /// `ttlMs: 0` / `cacheScope: "private"` on every cacheable family.
    ImmediatePrivate,
}

/// Product-owned policy for one advertised MCP protocol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtocolPolicy {
    pub(crate) version: ProtocolVersion,
    pub(crate) lifecycle: ProtocolLifecycle,
    pub(crate) cache: CachePolicy,
    pub(crate) resource_subscriptions: bool,
}

/// The complete product-supported protocol policy table, preferred first.
///
/// Single production source for advertised versions, lifecycle admission,
/// capability views, and cache-field shaping. `2025-11-25` is permanent
/// product compatibility and must not be removed by a future protocol or
/// rmcp update.
const SUPPORTED_PROTOCOLS: [ProtocolPolicy; 2] = [
    ProtocolPolicy {
        version: ProtocolVersion::V_2026_07_28,
        lifecycle: ProtocolLifecycle::DiscoverStateless,
        cache: CachePolicy::ImmediatePrivate,
        resource_subscriptions: true,
    },
    ProtocolPolicy {
        version: ProtocolVersion::V_2025_11_25,
        lifecycle: ProtocolLifecycle::InitializeSession,
        cache: CachePolicy::Omit,
        resource_subscriptions: false,
    },
];

/// Exact policy lookup. A version with no row (unknown, custom future, or
/// unsupported older) gets no policy — no lexical/date/range comparison.
pub(crate) fn policy_for(version: &ProtocolVersion) -> Option<&'static ProtocolPolicy> {
    SUPPORTED_PROTOCOLS.iter().find(|p| &p.version == version)
}

/// The preferred (first-row) policy. Total by construction: the table always
/// has at least one row, and `match` keeps this free of runtime indexing,
/// `unwrap`, or `expect`.
pub(crate) fn preferred_policy() -> &'static ProtocolPolicy {
    match &SUPPORTED_PROTOCOLS {
        [first, ..] => first,
    }
}

/// Ordered supported protocol versions, cloned from the policy table in
/// preferred order. The single production source for advertised versions;
/// there is deliberately no separate hand-maintained version list.
pub(crate) fn supported_protocol_versions() -> Vec<ProtocolVersion> {
    SUPPORTED_PROTOCOLS
        .iter()
        .map(|p| p.version.clone())
        .collect()
}

/// Whether a negotiated (optional) protocol version carries the SEP-2549
/// immediate-private cache fields (`ttlMs` / `cacheScope`). `None` and any
/// version without an exact policy row get no fields.
pub(crate) fn cache_fields_for(protocol_version: Option<ProtocolVersion>) -> bool {
    protocol_version
        .as_ref()
        .and_then(policy_for)
        .is_some_and(|p| p.cache == CachePolicy::ImmediatePrivate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_table_rows_are_exact_supported_versions_in_preferred_order() {
        let versions: Vec<ProtocolVersion> = supported_protocol_versions();
        assert_eq!(versions.len(), 2, "exactly two supported versions");
        assert_eq!(versions[0], ProtocolVersion::V_2026_07_28);
        assert_eq!(versions[1], ProtocolVersion::V_2025_11_25);
    }

    #[test]
    fn preferred_policy_is_modern_discovery_with_immediate_private_cache() {
        let preferred = preferred_policy();
        assert_eq!(preferred.version, ProtocolVersion::V_2026_07_28);
        assert_eq!(preferred.lifecycle, ProtocolLifecycle::DiscoverStateless);
        assert_eq!(preferred.cache, CachePolicy::ImmediatePrivate);
        assert!(
            preferred.resource_subscriptions,
            "modern row enables subscriptions"
        );
    }

    #[test]
    fn legacy_policy_is_initialize_session_with_omitted_cache() {
        let legacy = policy_for(&ProtocolVersion::V_2025_11_25).expect("legacy row must exist");
        assert_eq!(legacy.version, ProtocolVersion::V_2025_11_25);
        assert_eq!(legacy.lifecycle, ProtocolLifecycle::InitializeSession);
        assert_eq!(legacy.cache, CachePolicy::Omit);
        assert!(
            !legacy.resource_subscriptions,
            "legacy row disables subscriptions"
        );
    }

    #[test]
    fn older_known_unsupported_versions_have_no_policy() {
        for version in [
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2024_11_05,
        ] {
            assert!(
                policy_for(&version).is_none(),
                "{version} must have no policy row"
            );
        }
    }

    #[test]
    fn custom_future_version_has_no_policy_and_no_cache_fields() {
        // Deserialize is the only public constructor for custom versions.
        let future: ProtocolVersion =
            serde_json::from_value(serde_json::json!("2099-01-01")).unwrap();
        assert!(
            policy_for(&future).is_none(),
            "future version must have no policy row"
        );
        assert!(
            !cache_fields_for(Some(future)),
            "future version must not enable cache fields"
        );
    }

    #[test]
    fn cache_fields_follow_exact_policy_rows_only() {
        // No negotiated version: defensive, never occurs on the wire.
        assert!(!cache_fields_for(None));
        // Modern row carries the fields.
        assert!(cache_fields_for(Some(ProtocolVersion::V_2026_07_28)));
        // Legacy row omits them.
        assert!(!cache_fields_for(Some(ProtocolVersion::V_2025_11_25)));
        // Unsupported known versions get nothing despite being known to rmcp.
        assert!(!cache_fields_for(Some(ProtocolVersion::V_2025_06_18)));
        assert!(!cache_fields_for(Some(ProtocolVersion::V_2025_03_26)));
        assert!(!cache_fields_for(Some(ProtocolVersion::V_2024_11_05)));
    }
}
