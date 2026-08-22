//! Guards against tool-count drift between the code and its descriptions.
//!
//! The number of MCP tools is hardcoded in three prose surfaces (README.md,
//! Cargo.toml `description`, server.json `description`) and enumerated in the
//! README tool catalog table. The registry listing shipped "12 tools" while the
//! server had 22 because nothing tied these together — this test does.
//!
//! Source of truth: `serial_mcp::server::tool_catalog()`, the exact catalog the
//! MCP router serves (the README table, Cargo.toml, and server.json must all
//! agree with it). Detailed canonical contracts live in `docs/` guides
//! (`docs/rx-and-reading.md`, `docs/device-profiles.md`,
//! `docs/persistent-capture.md`); the README links them instead of
//! re-owning the prose.

use std::fs;
use std::path::Path;

fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Count `#[tool(` attribute occurrences in src/server.rs.
fn tool_count() -> usize {
    repo_file("src/server.rs").matches("#[tool(").count()
}

/// The exact tool-name set served by MCP, straight from the router's catalog.
fn served_tool_names() -> Vec<String> {
    serial_mcp::server::tool_catalog()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Extract every backtick-delimited inline-code span in `text`.
fn inline_code_spans(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut current = String::new();
    for c in text.chars() {
        if c == '`' {
            if in_code {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                in_code = false;
            } else {
                current.clear();
                in_code = true;
            }
        } else if in_code {
            current.push(c);
        }
    }
    out
}

#[test]
fn tool_count_is_sane() {
    // Guard the guard: if the attribute spelling ever changes, this fails
    // loudly instead of letting the other tests trivially pass on 0.
    assert!(
        tool_count() >= 10,
        "expected at least 10 #[tool( attributes in src/server.rs, found {} — \
         did the tool attribute syntax change?",
        tool_count()
    );
    assert_eq!(
        tool_count(),
        served_tool_names().len(),
        "the #[tool( attribute count must match the served catalog"
    );
}

#[test]
fn readme_tool_catalog_matches_served_catalog() {
    let expected = served_tool_names();
    let readme = repo_file("README.md");
    check_readme_tool_catalog(&readme, &expected)
        .unwrap_or_else(|e| panic!("README tool catalog drifted: {e}"));
}

/// The README tool catalog table is the human view of the exact served
/// catalog. Parse the "Tool catalog" section's inline-code identifiers and
/// compare as sets: no missing, no extra, no duplicates — and the section
/// heading must carry the visible count marker.
fn check_readme_tool_catalog(readme: &str, expected: &[String]) -> Result<(), String> {
    let n = expected.len();
    let marker = format!("({n} tools)");
    if !readme.contains(&marker) {
        return Err(format!(
            "README.md must show the '{marker}' count marker near its tool \
             catalog — the server serves {n} tools"
        ));
    }
    let start = readme
        .find("## Tool catalog")
        .ok_or_else(|| "README.md must have a '## Tool catalog' section".to_string())?;
    let rest = &readme[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let section = &rest[..end];
    let mut actual = inline_code_spans(section);
    actual.sort();
    let mut expected_sorted = expected.to_vec();
    expected_sorted.sort();
    if actual != expected_sorted {
        return Err(format!(
            "README tool catalog table must list exactly the served tools (no \
             missing, extra, or duplicate inline-code identifiers in that \
             section); expected {expected_sorted:?}, got {actual:?}"
        ));
    }
    Ok(())
}

#[test]
fn readme_tool_catalog_guard_rejects_dropped_tool() {
    // Negative proof: drop one tool's inline-code span from the catalog
    // section; the guard must name the set mismatch, not pass vacuously.
    let expected = served_tool_names();
    let readme = repo_file("README.md");
    let start = readme.find("## Tool catalog").expect("catalog section");
    let section_start = start + readme[start..].find('\n').unwrap_or(0) + 1;
    let end = readme[section_start..]
        .find("\n## ")
        .map(|i| section_start + i)
        .unwrap_or(readme.len());
    let section = &readme[section_start..end];
    let dropped = section
        .split("`")
        .nth(1)
        .expect("catalog section has an inline-code span")
        .to_string();
    let mutated = format!(
        "{}{}{}",
        &readme[..start],
        &readme[start..section_start],
        section.replacen(&format!("`{dropped}`"), "", 1)
    );
    let err = check_readme_tool_catalog(&mutated, &expected).unwrap_err();
    assert!(
        err.contains("exactly the served tools"),
        "guard failure must name the set mismatch: {err}"
    );
}

#[test]
fn cargo_toml_description_tool_count_matches_code() {
    let n = tool_count();
    let manifest = repo_file("Cargo.toml");
    let marker = format!("{n} tools");
    assert!(
        manifest.contains(&marker),
        "Cargo.toml description must contain {marker:?} — the server defines \
         {n} tools"
    );
}

#[test]
fn package_discovery_metadata_is_exact() {
    // crates.io/GitHub listings surface three discovery surfaces first: the
    // README H1 (line 1) and the Cargo [package] homepage/documentation
    // fields. They must stay exact — a drifted H1 or a wrong link reads
    // directly in registry and search-result listings.
    let readme = repo_file("README.md");
    let h1 = readme
        .lines()
        .next()
        .expect("README.md must have a first line");
    assert_eq!(
        h1, "# Serial MCP — UART and USB-Serial Access for AI Agents",
        "README line 1 is the package-discovery H1 and must stay exact"
    );

    let manifest = repo_file("Cargo.toml");
    let package_field = |field: &str| -> Option<&str> {
        manifest
            .lines()
            .find(|l| l.starts_with(&format!("{field} = ")))
    };
    assert_eq!(
        package_field("homepage"),
        Some("homepage = \"https://github.com/qarnet/serial-mcp\""),
        "Cargo.toml [package] homepage is package-discovery metadata and must stay exact"
    );
    assert_eq!(
        package_field("documentation"),
        Some("documentation = \"https://docs.rs/serial-mcp\""),
        "Cargo.toml [package] documentation is package-discovery metadata and must stay exact"
    );
}

#[test]
fn server_json_description_tool_count_matches_code() {
    let n = tool_count();
    let server_json = repo_file("server.json");
    let marker = format!("{n} tools");
    assert!(
        server_json.contains(&marker),
        "server.json description must contain {marker:?} — this string is \
         published verbatim to the MCP registry"
    );
}

#[test]
fn readme_mentions_every_protocol_preset() {
    // The preset list also drifts (the pre-0.7.1 README stopped at SLIP).
    // Keep this list in sync with `ProtocolPreset` in src/framing/config.rs; the
    // enum source is grepped so adding a preset without README mention fails.
    let framing = repo_file("src/framing/config.rs");
    let readme = repo_file("README.md");
    let enum_body = framing
        .split("pub enum ProtocolPreset")
        .nth(1)
        .and_then(|s| s.split('}').next())
        .expect("src/framing/config.rs must define ProtocolPreset");
    // Serde renames variants to snake_case; derive the wire names.
    let wire_names: Vec<String> = enum_body
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("///")
                && !l.starts_with("//")
                && !l.starts_with('#')
                && l.ends_with(',')
        })
        .map(|l| {
            let variant = l.trim_end_matches(',');
            let mut out = String::new();
            for (i, c) in variant.char_indices() {
                if c.is_ascii_uppercase() {
                    if i > 0 {
                        out.push('_');
                    }
                    out.push(c.to_ascii_lowercase());
                } else if c.is_ascii_digit() || c.is_ascii_lowercase() {
                    out.push(c);
                } else {
                    // Non-identifier char: not a plain variant line.
                    return String::new();
                }
            }
            out
        })
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        wire_names.len() >= 7,
        "expected at least 7 ProtocolPreset variants, parsed {wire_names:?}"
    );
    for name in &wire_names {
        assert!(
            readme.contains(&format!("`{name}`")),
            "README.md must mention protocol preset `{name}`"
        );
    }
}

#[test]
fn rx_guide_readfrom_examples_use_tagged_wire_form() {
    // The `ReadFrom` wire format is a tagged object (`{"type":"now"}`), not a
    // bare string. The canonical contract lives in docs/rx-and-reading.md; the
    // README links that guide. Prose that teaches the `from` parameter must
    // not regress to string shorthand — agents copy these examples verbatim.
    let guide = repo_file("docs/rx-and-reading.md");
    for tagged in [
        r#"{"type":"cursor"}"#,
        r#"{"type":"now"}"#,
        r#"{"type":"buffer_start"}"#,
        r#"{"type":"offset","offset":N}"#,
    ] {
        assert!(
            guide.contains(tagged),
            "docs/rx-and-reading.md must contain the tagged `from` form {tagged}"
        );
    }
    for shorthand in [
        "from: \"now\"",
        "from: \"cursor\"",
        "from: \"buffer_start\"",
    ] {
        assert!(
            !guide.contains(shorthand),
            "docs/rx-and-reading.md must not advertise {shorthand}"
        );
    }
    let readme = repo_file("README.md");
    assert!(
        readme.contains("docs/rx-and-reading.md"),
        "README.md must link the RX/reading guide that owns the `from` contract"
    );
}

#[test]
fn server_json_versions_match_cargo_toml() {
    let cargo_version = cargo_toml_version();
    let server_json = repo_file("server.json");
    let mismatches = server_json_version_mismatches(&server_json, &cargo_version)
        .expect("server.json version fields must be readable");
    assert!(
        mismatches.is_empty(),
        "server.json version fields {mismatches:?} do not match Cargo.toml \
         package version {cargo_version:?}"
    );
}

#[test]
fn changelog_matches_cargo_package_version() {
    // Release roll contract: a Cargo.toml package version bump must come with
    // a release table row and a matching body heading in CHANGELOG.md, plus
    // an [Unreleased] heading above the current release. Reads the real
    // Cargo.toml + CHANGELOG.md.
    let version = cargo_toml_version();
    let changelog = repo_file("CHANGELOG.md");
    check_changelog_contract(&changelog, &version)
        .unwrap_or_else(|e| panic!("release/documentation consistency violated: {e}"));
}

#[test]
fn changelog_contract_rejects_missing_version_table_row() {
    // Negative proof: drop only the current-version table row from a
    // contract-satisfying fixture; the failure must name the table row, not
    // the body headings.
    let version = "9.9.9";
    let row = format!("| [{version}](#{}) |", version.replace('.', ""));
    let text = synthetic_changelog(version)
        .lines()
        .filter(|l| !l.starts_with(&row))
        .collect::<Vec<_>>()
        .join("\n");
    let err = check_changelog_contract(&text, version).unwrap_err();
    assert_eq!(
        err,
        format!(
            "CHANGELOG release table must contain a row starting with {row:?} \
             for version {version:?}"
        )
    );
}

#[test]
fn changelog_contract_rejects_missing_version_heading() {
    // Negative proof: drop only the `## [x.y.z]` body heading; the table row
    // stays, so the failure must be the heading rule alone.
    let version = "9.9.9";
    let text = synthetic_changelog(version)
        .lines()
        .filter(|l| *l != format!("## [{version}]"))
        .collect::<Vec<_>>()
        .join("\n");
    let err = check_changelog_contract(&text, version).unwrap_err();
    assert_eq!(
        err,
        format!("CHANGELOG body must contain the exact heading \"## [{version}]\"")
    );
}

#[test]
fn changelog_contract_rejects_missing_unreleased_heading() {
    // Negative proof: drop only the `## [Unreleased]` heading.
    let version = "9.9.9";
    let text = synthetic_changelog(version)
        .lines()
        .filter(|l| *l != "## [Unreleased]")
        .collect::<Vec<_>>()
        .join("\n");
    let err = check_changelog_contract(&text, version).unwrap_err();
    assert_eq!(
        err,
        "CHANGELOG body must contain the heading \"## [Unreleased]\""
    );
}

#[test]
fn changelog_contract_rejects_unreleased_after_current_release() {
    // Negative proof: swap the two headings so [Unreleased] sits after the
    // current release; every earlier rule still passes, so only the ordering
    // rule may fire.
    let version = "9.9.9";
    let heading = format!("## [{version}]");
    let unreleased = "## [Unreleased]";
    let fixture = synthetic_changelog(version);
    let mut lines: Vec<&str> = fixture.lines().collect();
    let u = lines.iter().position(|l| *l == unreleased).unwrap();
    let r = lines.iter().position(|l| *l == heading).unwrap();
    lines.swap(u, r);
    let err = check_changelog_contract(&lines.join("\n"), version).unwrap_err();
    assert_eq!(
        err,
        format!(
            "the \"## [Unreleased]\" heading must appear before the \
             \"## [{version}]\" heading"
        )
    );
}

#[test]
fn server_json_version_mismatch_is_rejected() {
    // Negative proof for the Cargo/server.json version equality: a committed
    // template with a drifted version field must be reported with the
    // mismatching value, not silently accepted.
    let cargo_version = cargo_toml_version();
    let server_json = repo_file("server.json");
    let v: serde_json::Value =
        serde_json::from_str(&server_json).expect("server.json is valid JSON");
    let mut obj = v
        .as_object()
        .expect("server.json top-level value must be an object")
        .clone();
    obj.insert(
        "version".to_string(),
        serde_json::Value::String("9.9.9-drift".to_string()),
    );
    let drifted = serde_json::Value::Object(obj).to_string();
    let mismatches = server_json_version_mismatches(&drifted, &cargo_version)
        .expect("drifted template still yields readable version fields");
    assert_eq!(
        mismatches,
        vec!["9.9.9-drift"],
        "drifted server.json version must be reported"
    );
}

#[test]
fn server_json_omits_packages() {
    // The committed server.json is a registry template: the packages array
    // (release-asset URLs + fileSha256) is generated at publish time by
    // .github/workflows/publish-mcp-registry.yml from the actual release
    // binaries. A committed packages array goes stale on every release —
    // 0.5.1 URLs and hashes survived in the repo until 0.7.3 — so it must
    // not exist here.
    let server_json = repo_file("server.json");
    let v: serde_json::Value =
        serde_json::from_str(&server_json).expect("server.json is valid JSON");
    assert!(
        v.get("packages").is_none(),
        "server.json must not contain a committed \"packages\" array — it is \
         generated per-release by publish-mcp-registry.yml (committed entries \
         inevitably go stale)"
    );
}

#[test]
fn readme_teaches_capture_boot_boot_path_and_semantics() {
    // The README must teach capture_boot as the boot/reset path,
    // and the tool description must state the private cursor, OS-input
    // purge, optional line pulse, bounded in-memory result, and no file
    // output.
    let readme = repo_file("README.md");
    assert!(
        readme.contains("capture_boot"),
        "README must teach capture_boot as the boot/reset path"
    );
    let server = repo_file("src/server.rs");
    let desc_start = server
        .find("Atomic boot/reset capture")
        .expect("capture_boot tool description");
    let desc = &server[desc_start..desc_start + 1200];
    for needle in [
        "private read cursor",
        "purges unread OS input",
        "pulses DTR/RTS",
        "no file output",
        "bounded in memory",
    ] {
        assert!(
            desc.contains(needle),
            "capture_boot tool description must state {needle:?}"
        );
    }
    // The decision-tree instructions teach the boot path too.
    let instructions = server
        .split("with_instructions(")
        .nth(1)
        .and_then(|s| s.split("to_string()").next())
        .expect("server.rs must contain with_instructions");
    assert!(
        instructions.contains("capture_boot"),
        "server instructions must teach capture_boot"
    );
}

#[test]
fn readme_teaches_profile_discovery_and_common_flow() {
    // The normal workflow is discover → bare open → transact →
    // inspect the learned profile → escalate. Positive guidance assertions.
    let readme = repo_file("README.md");
    assert!(
        readme.contains("profile_matches"),
        "README must teach the list_ports profile-match preview"
    );
    assert!(
        readme.contains("transact"),
        "README must teach transact as the command/response primitive"
    );
    assert!(
        readme.contains("bare `open`") || readme.contains("bare open"),
        "README must teach bare open as the common call"
    );
    assert!(
        readme.contains("profile_persistence"),
        "README must teach inspecting profile persistence after durable changes"
    );
}

#[test]
fn prompts_teach_current_decision_tree_without_stale_references() {
    // The diagnose prompt must teach list_ports → bare open → transact →
    // rollback, and neither prompt may reference removed tools or removed
    // per-call fields.
    let diagnose = repo_file("src/prompts/diagnose.rs");
    assert!(
        diagnose.contains("`list_ports`") && diagnose.contains("profile_matches"),
        "diagnose prompt must teach the profile-match preview"
    );
    assert!(
        diagnose.contains("`open(port="),
        "diagnose prompt must teach the bare open call"
    );
    assert!(
        diagnose.contains("`transact("),
        "diagnose prompt must use transact for probes"
    );
    assert!(
        diagnose.contains("rollback_profile"),
        "diagnose prompt must teach rollback after bad learned settings"
    );
    assert!(
        diagnose.contains("capture_boot"),
        "diagnose prompt must teach capture_boot for boot/reset capture"
    );
    let interactive = repo_file("src/prompts/interactive.rs");
    assert!(
        interactive.contains("`transact("),
        "interactive prompt must drive commands via transact"
    );
    for prompt_src in [&diagnose, &interactive] {
        assert!(
            !prompt_src.contains("wait_for"),
            "prompts must not reference the removed wait_for tool"
        );
        assert!(
            !prompt_src.contains("max_buffered_bytes"),
            "prompts must not use the removed per-call max_buffered_bytes"
        );
    }
}

#[test]
fn server_instructions_teach_decision_tree() {
    // The server `instructions` string (served on initialize) must carry the
    // decision tree, not a flat tool list.
    let server = repo_file("src/server.rs");
    let instructions = server
        .split("with_instructions(")
        .nth(1)
        .and_then(|s| s.split("to_string()").next())
        .expect("server.rs must contain with_instructions");
    for needle in [
        "list_ports",
        "bare",
        "transact",
        "profile_matches",
        "rollback_profile",
        "open_profile",
    ] {
        assert!(
            instructions.contains(needle),
            "server instructions must teach {needle}"
        );
    }
}

#[test]
fn agent_config_rx_troubleshooting_links_rx_guide() {
    // docs/agent-config.md referenced the removed `#how-rx-works` anchor; the
    // RX model contract now lives in docs/rx-and-reading.md and the
    // troubleshooting entry must point there, with the target file existing.
    let config = repo_file("docs/agent-config.md");
    assert!(
        !config.contains("#how-rx-works"),
        "agent-config.md must not reference the removed README anchor"
    );
    assert!(
        config.contains("rx-and-reading.md"),
        "agent-config.md must link the RX/reading guide"
    );
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/rx-and-reading.md");
    assert!(
        target.is_file(),
        "agent-config.md must link an existing docs/rx-and-reading.md"
    );
}

#[test]
fn features_md_does_not_relist_shipped_items() {
    // FEATURES.md is the roadmap/tech-debt file only. Shipped items
    // (configure, transact, compute_checksum, reconnect policy, ...) must not
    // be re-added there — CHANGELOG.md and AGENTS.md own shipped truth, and a
    // shipped marker in the roadmap reads as unbuilt or goes stale.
    let features = repo_file("docs/development/FEATURES.md");
    assert!(
        !features.contains("✅ **Shipped"),
        "FEATURES.md must not relist shipped items with a shipped marker"
    );
    assert!(
        !features.contains("pure wiring: profile field"),
        "FEATURES.md must not still describe the reconnect-policy item as unwired"
    );
}

#[test]
fn consumed_implementation_handoffs_are_removed() {
    // docs/development/github-actions-security-handoff.md documented GitHub
    // Actions security hardening that has since shipped. Development policy
    // says consumed implementation handoffs are removed from the tree (git
    // history preserves them) — a surviving handoff reads as pending work.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/development/github-actions-security-handoff.md");
    assert!(
        !path.is_file(),
        "consumed implementation handoffs must be removed from the tree; \
         delete docs/development/github-actions-security-handoff.md (git \
         history preserves the content)"
    );
}

#[test]
fn features_md_removed_roadmap_headings_stay_removed() {
    // The old "Positive MCP cache hints" heading described the shipped
    // ttlMs=0/private baseline as if it were an unbuilt feature, and
    // "Multiple public subscriptions per connection" predated removal of the
    // serial subscribe/unsubscribe tools. Neither may reappear, and the
    // Per-client RX cursors item must not resurrect the removed cross-link.
    let features = repo_file("docs/development/FEATURES.md");
    for heading in [
        "### Positive MCP cache hints",
        "### Multiple public subscriptions per connection",
    ] {
        assert!(
            !features.contains(heading),
            "FEATURES.md must not contain the removed roadmap heading {heading:?}"
        );
    }
    let cursors = features
        .split("### Per-client RX cursors")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .unwrap_or_default();
    assert!(
        !cursors.contains("Multiple public subscriptions"),
        "Per-client RX cursors must not cross-reference the removed \
         subscription heading"
    );
}

#[test]
fn features_md_cache_item_distinguishes_shipped_baseline_from_future_policy() {
    // The roadmap item is ONLY the future positive-TTL policy: the shipped
    // 2026-07-28 cache baseline (ttlMs=0, private) must be named as shipped,
    // never as the unbuilt feature. Checked semantically so paragraph layout
    // can change.
    let features = repo_file("docs/development/FEATURES.md");
    let item = features
        .split("### Positive MCP cache TTL policy")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .unwrap_or_default();
    assert!(
        item.contains("ttlMs=0") && item.contains("private"),
        "the future cache-TTL item must name the shipped non-cacheable \
         baseline (ttlMs=0, private)"
    );
    assert!(
        item.contains("future") || item.contains("NOT shipped") || item.contains("if pursued"),
        "the cache-TTL item must frame positive TTL as future, not shipped: {item}"
    );
    for prereq in [
        "invalidation",
        "authorization partitioning",
        "pagination keys",
        "stale-on-error",
    ] {
        assert!(
            item.contains(prereq),
            "the future cache-TTL item must keep prerequisite {prereq:?}"
        );
    }
}

#[test]
fn features_md_baud_detection_stays_deferred_and_keeps_expliot_reference() {
    // Baud-rate auto-detection is explicitly deferred: generic host-side
    // detection over a USB-serial adapter is heuristic, not waveform
    // measurement, so the roadmap item must stay marked deferred. The
    // EXPLIoT reference is a pointer to an existing solution — guard the
    // deferred marker and the exact repository URL so neither drifts.
    let features = repo_file("docs/development/FEATURES.md");
    let item = features
        .split("### Baud-rate auto-detection")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .unwrap_or_default();
    assert!(
        item.starts_with(" *(deferred)*"),
        "the baud-rate auto-detection heading must be marked explicitly deferred"
    );
    assert!(
        item.contains("https://gitlab.com/expliot_framework/expliot"),
        "the baud-rate item must retain the exact EXPLIoT repository URL"
    );
}

#[test]
fn readme_links_capture_guide_with_short_summary() {
    // The README owns a one-sentence summary of the capture contract and links
    // the canonical guide; the detailed contract lives in
    // docs/persistent-capture.md.
    let readme = repo_file("README.md");
    assert!(
        readme.contains("--capture-dir"),
        "README must mention --capture-dir"
    );
    assert!(
        readme.contains("persistent-capture.md"),
        "README must link the persistent-capture guide"
    );
    let summary = readme
        .find("**Persistent capture:**")
        .map(|i| &readme[i..i + 600])
        .unwrap_or_default();
    assert!(
        summary.contains(".jsonl"),
        "README capture summary must say the export path is a portable filename: {summary}"
    );
    assert!(
        summary.contains("never overwrites") || summary.contains("no overwrite"),
        "README capture summary must state the no-overwrite rule: {summary}"
    );
}

#[test]
fn capture_guide_documents_full_export_contract() {
    // docs/persistent-capture.md is the canonical owner of the detailed
    // contract: disabled by default, the exact quota options, filename-only
    // portable paths, no-overwrite atomic snapshots, the advisory lock, and
    // failure/durability semantics.
    let guide = repo_file("docs/persistent-capture.md");
    for needle in [
        "--capture-dir",
        "--capture-max-file-bytes",
        "--capture-max-total-bytes",
        "--capture-max-files",
        "filename",
        "never overwrites",
        "point-in-time",
        "advisory",
        "durability_warning",
        "Trust boundary",
    ] {
        assert!(
            guide.contains(needle),
            "docs/persistent-capture.md must document {needle:?}"
        );
    }
}

#[test]
fn rx_guide_states_ring_capacity_and_rejects_removed_subscribe() {
    // docs/rx-and-reading.md is the canonical RX contract. The ring's
    // retention capacity is rx_buffer_size (fixed at open); max_buffered_bytes
    // is the per-read/in-memory result cap, not the ring bound. The removed
    // `subscribe` tool must not be suggested or code-formatted.
    let guide = repo_file("docs/rx-and-reading.md");
    let paragraphs: Vec<&str> = guide.split("\n\n").collect();
    let rx_para = paragraphs
        .iter()
        .find(|p| p.contains("`rx_buffer_size`"))
        .expect("docs/rx-and-reading.md must name rx_buffer_size");
    assert!(
        rx_para.contains("fixed at open") || rx_para.contains("retention"),
        "guide must tie rx_buffer_size to ring retention/capacity: {rx_para}"
    );
    let max_para = paragraphs
        .iter()
        .find(|p| p.contains("max_buffered_bytes"))
        .expect("docs/rx-and-reading.md must name max_buffered_bytes");
    assert!(
        max_para.contains("read-result cap"),
        "guide must call max_buffered_bytes the read/result cap, not the ring \
         bound: {max_para}"
    );
    assert!(
        !guide.contains("subscribe-style"),
        "docs/rx-and-reading.md must not suggest subscribe-style monitoring \
         (the subscribe tool was removed)"
    );
    assert!(
        !guide.contains("`subscribe`"),
        "docs/rx-and-reading.md must not reference a code-formatted removed \
         `subscribe` tool"
    );
}

#[test]
fn device_profiles_guide_states_none_outcomes_and_explicit_weak() {
    // docs/device-profiles.md must distinguish the two `none` outcomes (high
    // unique bare open generates a profile; weak/path-only starts transient)
    // and keep explicit open_profile available for weak identity.
    let guide = repo_file("docs/device-profiles.md");
    let none_line = guide
        .lines()
        .find(|l| l.contains("| `none` |"))
        .expect("device-profiles.md must document the `none` outcome");
    assert!(
        none_line.contains("generated") && none_line.contains("transient"),
        "the `none` outcome must cover high-generated AND weak-transient \
         behavior: {none_line}"
    );
    // One paragraph must tie explicit open_profile to weak identity — an
    // independent document-wide mention of both would not prove the
    // qualification is stated together.
    let weak_para = guide
        .split("\n\n")
        .find(|p| p.contains("open_profile") && p.contains("weak"))
        .expect(
            "device-profiles.md must qualify explicit open_profile for \
             weak identity in the same passage",
        );
    assert!(
        weak_para.contains("explicit"),
        "the weak-identity passage must frame open_profile as the explicit \
         path (as opposed to the automatic one): {weak_para}"
    );
}

#[test]
fn docs_index_links_new_guides_and_targets_exist() {
    // docs/README.md is the user-documentation index: it must link the three
    // new user guides, agent config, the protocol guide, the development
    // index, and the roadmap, and every relative target must exist on disk.
    let index = repo_file("docs/README.md");
    let targets = [
        "agent-config.md",
        "rx-and-reading.md",
        "device-profiles.md",
        "persistent-capture.md",
        "protocols.md",
        "development/README.md",
        "development/FEATURES.md",
    ];
    for target in targets {
        assert!(
            index.contains(&format!("]({target})")),
            "docs/README.md must link {target:?}"
        );
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs")
            .join(target);
        assert!(
            path.is_file(),
            "docs/README.md links docs/{target} but that file does not exist"
        );
    }
}

#[test]
fn capture_cli_options_synced_between_value_list_and_help() {
    // The VALUE_TAKING_OPTIONS const and the --help block must both list
    // every capture option, or `--capture-dir --version` style detection
    // silently drifts (see the CROSS-REFERENCE comment in main.rs).
    let main = repo_file("src/main.rs");
    let value_list = main
        .split("const VALUE_TAKING_OPTIONS: &[&str] = &[")
        .nth(1)
        .and_then(|s| s.split("];").next())
        .unwrap_or_default();
    let help_block = main
        .split("Usage: serial-mcp [OPTIONS]")
        .nth(1)
        .and_then(|s| s.split("Commands:").next())
        .unwrap_or_default();
    for opt in [
        "--capture-dir",
        "--capture-max-file-bytes",
        "--capture-max-total-bytes",
        "--capture-max-files",
    ] {
        assert!(
            value_list.contains(opt),
            "VALUE_TAKING_OPTIONS must contain {opt}"
        );
        assert!(help_block.contains(opt), "--help block must document {opt}");
    }
}

#[test]
fn native_replacement_plan_requires_public_pty_parity_before_removal() {
    let development_index = repo_file("docs/development/README.md");
    assert!(development_index.contains(
        "[native-sim-replacement-research-plan.md](native-sim-replacement-research-plan.md)"
    ));
    let research = repo_file("docs/development/native-sim-replacement-research-plan.md");
    for needle in [
        "43",
        "6",
        "all seven shipped protocol presets",
        "Hard rejection criteria",
        "Required Proof-of-Concept Experiments",
        "Step-by-Step Research TODO",
        "Remove native_sim and NCS completely",
        "Full required suite passes from clean checkout",
    ] {
        assert!(
            research.contains(needle),
            "native_sim replacement research plan must retain {needle:?}"
        );
    }
    for artifact in [
        "native-sim-test-traceability.md",
        "native-sim-virtual-serial-candidate-survey.md",
        "native-sim-boundary-prototype-results.md",
        "native-sim-emulator-core-research.md",
        "native-sim-protocol-peer-worksheets.md",
        "native-sim-replacement-recommendation.md",
        "replace-native-sim-with-rust-pty-device-fixture.md",
    ] {
        assert!(
            research.contains(artifact),
            "executed research plan must link {artifact:?}"
        );
    }
    assert!(
        research.contains("Recommendation approved and Phases A-E implemented")
            && research.contains("peer-master closure")
            && research.contains("Phase A resolved")
            && research.contains("`connection_closed`")
            && research.contains("Full differential outcome comparison")
            && research.contains("precedes Phase F"),
        "research plan must preserve initial blocker evidence and current migration status"
    );
    assert!(
        !research.contains("awaits user approval"),
        "research plan must not describe the approved replacement as awaiting approval"
    );
}

const NATIVE_TRACEABILITY_ROWS: [&str; 49] = [
    "native_ping_roundtrip",
    "native_pending_read_then_write_ping_roundtrip",
    "native_split_writes_preserve_command_order",
    "native_framing_reports_single_split_command",
    "native_trace_reports_exact_split_byte_sequence",
    "native_read_match_on_spam_complete",
    "native_read_buffer_budget_stops_under_flood",
    "native_bootloader_touch_exits_42",
    "native_list_ports_after_open",
    "native_list_ports_includes_identity_fields",
    "native_flush_after_write",
    "native_get_status_after_write_increments_tx_counter",
    "native_reconfigure_baud_rate_persists",
    "native_ack_command_provides_pre_execution_ack",
    "native_txbuf_status_reports_pending",
    "native_flush_input_clears_host_rx",
    "native_flush_during_arm_cmd_delay",
    "native_flush_output_after_full_delivery_is_safe",
    "native_partial_line_buffered_then_completed",
    "native_read_regex_matches_pong",
    "native_read_glob_matches_pong_line",
    "native_auto_reconnect_preserves_connection",
    "native_read_line_framing_splits_lines",
    "native_read_json_parser_decodes_jsonout",
    "native_read_at_parser_parses_pong",
    "native_read_framing_max_frames_stops",
    "native_read_framing_plus_match_combined",
    "native_open_protocol_default_drives_write_and_read",
    "native_explicit_rx_framing_beats_connection_default",
    "native_read_slip_decodes_frame",
    "native_read_slip_malformed_escape_returns_partial_result",
    "native_read_delimiter_framing_decodes",
    "native_read_length_prefixed_framing_decodes",
    "native_read_start_end_framing_decodes",
    "native_write_tx_framing_modes_observed_via_trace",
    "native_read_explicit_line_endings_split_correctly",
    "native_read_slip_recovers_after_error_on_next_call",
    "native_read_cobs_preset_decodes_frame",
    "native_read_ndjson_preset_decodes_json_frames",
    "native_read_ndjson_preset_skips_empty_lines",
    "native_read_nmea0183_preset_decodes_parsed_frame",
    "native_read_modbus_ascii_preset_decodes_parsed_frame",
    "native_capture_boot_arm_only_captures_post_arm_command_output",
    "native_named_connection_appears_in_list_connections",
    "native_set_flow_control_updates_summary_and_result",
    "native_close_while_read_active_returns_normal_result",
    "native_reopen_same_port_after_close_works",
    "native_reopen_then_match_finds_fresh_output",
    "native_open_with_flow_control_persists_in_summary",
];

const TRACEABILITY_REPLACEMENT_IDENTIFIERS: [&str; 35] = [
    "ping_roundtrip_uses_real_path_and_literal_match",
    "pending_read_receives_later_output_after_readiness_proven_hold",
    "split_writes_preserve_one_command_and_exact_wire_order",
    "named_connection_summary_uses_fixture_stable_path",
    "reopen_same_path_returns_distinct_id_and_only_fresh_generation",
    "status_reports_exact_io_deltas_and_activity",
    "reconfigure_updates_status_and_connection_remains_functional",
    "ack_peer_orders_ack_before_response_and_stops_after_disable",
    "held_output_reports_nonzero_queue_then_drains_and_recovers",
    "flush_input_discards_known_old_marker_and_keeps_new_marker",
    "flush_after_command_acceptance_does_not_cancel_delayed_response",
    "output_flush_after_full_delivery_preserves_later_traffic",
    "regex_and_glob_matchers_find_complete_peer_line",
    "line_framing_returns_exact_ordered_peer_frames",
    "max_frames_stops_after_exact_limit",
    "framing_plus_match_returns_matching_frame_and_index",
    "call_time_line_framing_beats_connection_delimiter_default",
    "delimiter_length_prefixed_and_start_end_decode_exact_payloads",
    "explicit_line_endings_split_with_documented_terminator_semantics",
    "tx_framing_modes_produce_exact_independent_wire_vectors",
    "json_lines_preset_writes_line_and_preserves_object_only_parser_behavior",
    "at_command_connection_default_drives_stateful_transact_and_parser_quirk",
    "slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery",
    "cobs_preset_uses_independent_zero_byte_vector_for_write_and_read",
    "ndjson_preset_parses_records_and_skips_blank_whitespace_lines",
    "nmea0183_preset_parses_valid_independently_checksummed_sentence",
    "modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation",
    "finite_flood_matcher_reaches_unique_completion_marker",
    "live_buffer_budget_caps_finite_flood_with_exact_stop_metadata",
    "public_mcp_ping_hold_disconnect_replace_and_reconnect",
    "touch_write_causes_small_rust_child_peer_to_exit_42",
    "flow_control_none_at_open_and_live_set_are_reflected_in_summary",
    "close_interrupts_readiness_proven_live_read_with_connection_closed",
    "capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor",
    "phase_e_public_boundary_repeat_gate",
];

/// Existing public tests that strengthen a non-retired native case. They are
/// deliberately separate from retirement-only proofs so a retired-proof ID
/// cannot become a blanket substitute for required replacement coverage.
const TRACEABILITY_STRENGTHENED_EXISTING_PUBLIC_PROOF_IDENTIFIERS: [&str; 3] = [
    "list_ports_preview_empty_store_reports_none_parallel_and_pure_ports",
    "list_ports_preview_selected_winner_matches_later_bare_open",
    "list_ports_preview_output_validates_against_generated_schema",
];

/// Existing public tests usable only to justify an explicit retired row.
const TRACEABILITY_RETIREMENT_ONLY_PROOF_IDENTIFIERS: [&str; 2] = [
    "call_tool_list_ports_returns_structured_result",
    "ports_resource_includes_profile_match_map",
];

const RETIRED_NATIVE_TRACEABILITY_ROWS: [&str; 3] = [
    "native_list_ports_after_open",
    "native_flush_after_write",
    "native_reopen_same_port_after_close_works",
];

fn traceability_mapping_rows(traceability: &str) -> Result<Vec<&str>, String> {
    let start = traceability
        .find("## Implemented Replacement Mapping")
        .ok_or_else(|| "traceability document lacks implemented mapping heading".to_string())?;
    let rest = &traceability[start..];
    let end = rest
        .find("\n## Disposition Rules")
        .ok_or_else(|| "traceability mapping lacks disposition heading boundary".to_string())?;
    Ok(rest[..end]
        .lines()
        .filter(|line| line.starts_with("| `native_"))
        .collect())
}

fn traceability_mapping_cells(row: &str) -> Result<(&str, &str, &str), String> {
    let cells: Vec<_> = row.trim().split('|').map(str::trim).collect();
    if cells.len() != 5 || !cells[0].is_empty() || !cells[4].is_empty() {
        return Err(format!("mapping row must have three table cells: {row}"));
    }
    Ok((cells[1], cells[2], cells[3]))
}

fn mapping_native_identifier(cell: &str) -> Result<String, String> {
    let identifiers = inline_code_spans(cell);
    if identifiers.len() != 1 || !identifiers[0].starts_with("native_") {
        return Err(format!(
            "mapping native cell must contain exactly one native test identifier: {cell}"
        ));
    }
    Ok(identifiers[0].clone())
}

fn mapping_replacement_identifiers(cell: &str) -> Vec<String> {
    inline_code_spans(cell)
        .into_iter()
        .map(|identifier| {
            identifier
                .rsplit("::")
                .next()
                .unwrap_or(identifier.as_str())
                .to_owned()
        })
        .collect()
}

fn source_test_identifier_exists(identifier: &str) -> bool {
    [
        "tests/device_fixture.rs",
        "tests/device_command_parity.rs",
        "tests/device_framing_parity.rs",
        "tests/device_protocol_parity.rs",
        "tests/device_parity_repeat.rs",
        "tests/http_integration.rs",
        "tests/serial_pty.rs",
    ]
    .into_iter()
    .any(|path| repo_file(path).contains(&format!("fn {identifier}")))
}

fn native_source_test_exists(identifier: &str) -> bool {
    [
        "tests/native_sim_validation/unix.rs",
        "tests/native_sim_connection_lifecycle.rs",
    ]
    .into_iter()
    .any(|path| repo_file(path).contains(&format!("fn {identifier}")))
}

fn is_non_retired_traceability_proof(identifier: &str) -> bool {
    TRACEABILITY_REPLACEMENT_IDENTIFIERS.contains(&identifier)
        || TRACEABILITY_STRENGTHENED_EXISTING_PUBLIC_PROOF_IDENTIFIERS.contains(&identifier)
}

#[test]
fn native_traceability_mapping_is_exact_and_fixture_backed() {
    // Expected names stay independent of source discovery. This catches a
    // deleted/renamed native row, duplicate table row, stale/unknown mapping,
    // or a cited replacement/retirement proof whose source test disappeared.
    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    let rows = traceability_mapping_rows(&traceability)
        .unwrap_or_else(|error| panic!("native traceability mapping invalid: {error}"));
    assert_eq!(
        rows.len(),
        NATIVE_TRACEABILITY_ROWS.len(),
        "native traceability mapping must contain exactly {} rows",
        NATIVE_TRACEABILITY_ROWS.len()
    );
    let mut mapped_native_names = Vec::with_capacity(rows.len());
    let mut mapped_replacement_identifiers = std::collections::BTreeSet::new();
    let mut retired_native_names = std::collections::BTreeSet::new();
    let known_replacements: std::collections::BTreeSet<_> = TRACEABILITY_REPLACEMENT_IDENTIFIERS
        .iter()
        .chain(TRACEABILITY_STRENGTHENED_EXISTING_PUBLIC_PROOF_IDENTIFIERS.iter())
        .chain(TRACEABILITY_RETIREMENT_ONLY_PROOF_IDENTIFIERS.iter())
        .copied()
        .collect();
    for row in &rows {
        let (native_cell, replacement_cell, evidence_cell) = traceability_mapping_cells(row)
            .unwrap_or_else(|error| panic!("native traceability row invalid: {error}"));
        let native = mapping_native_identifier(native_cell)
            .unwrap_or_else(|error| panic!("native traceability row invalid: {error}"));
        assert!(
            NATIVE_TRACEABILITY_ROWS.contains(&native.as_str()),
            "native traceability mapping contains unknown native case {native:?}: {row}"
        );
        mapped_native_names.push(native);
        let is_retired = evidence_cell.contains("**Retired.**");
        if is_retired {
            retired_native_names.insert(
                mapping_native_identifier(native_cell)
                    .expect("mapping native cell was validated above"),
            );
        }
        let replacements = mapping_replacement_identifiers(replacement_cell);
        assert!(
            !replacements.is_empty(),
            "mapping row must cite at least one replacement or retirement proof: {row}"
        );
        for replacement in &replacements {
            assert!(
                known_replacements.contains(replacement.as_str()),
                "mapping row cites unknown replacement/retirement proof {replacement:?}: {row}"
            );
            assert!(
                source_test_identifier_exists(replacement),
                "mapping row cites replacement/retirement proof {replacement:?} with no source test"
            );
            mapped_replacement_identifiers.insert(replacement.clone());
        }
        assert!(
            is_retired
                || replacements
                    .iter()
                    .all(|identifier| is_non_retired_traceability_proof(identifier)),
            "non-retired mapping row may cite only required replacement identifiers or \
             strengthened existing public proofs: {row}"
        );
    }
    for native in NATIVE_TRACEABILITY_ROWS {
        assert!(
            native_source_test_exists(native),
            "native traceability expected name {native:?} has no source test"
        );
        let occurrences = mapped_native_names
            .iter()
            .filter(|actual| actual.as_str() == native)
            .count();
        assert_eq!(
            occurrences, 1,
            "native traceability mapping must represent {native:?} exactly once; mapped={mapped_native_names:?}"
        );
    }
    assert_eq!(
        retired_native_names,
        RETIRED_NATIVE_TRACEABILITY_ROWS
            .into_iter()
            .map(str::to_owned)
            .collect(),
        "traceability must keep exactly the three explicit retirement rows"
    );
    for identifier in TRACEABILITY_REPLACEMENT_IDENTIFIERS {
        assert!(
            traceability.contains(identifier),
            "traceability document must cite replacement identifier {identifier:?}"
        );
        assert!(
            source_test_identifier_exists(identifier),
            "traceability replacement identifier {identifier:?} has no source test"
        );
        if identifier != "phase_e_public_boundary_repeat_gate" {
            assert!(
                mapped_replacement_identifiers.contains(identifier),
                "traceability mapping rows must cite replacement identifier {identifier:?}"
            );
        }
    }
    for identifier in TRACEABILITY_STRENGTHENED_EXISTING_PUBLIC_PROOF_IDENTIFIERS {
        assert!(
            traceability.contains(identifier),
            "traceability document must cite strengthened existing public proof {identifier:?}"
        );
        assert!(
            source_test_identifier_exists(identifier),
            "traceability strengthened existing public proof {identifier:?} has no source test"
        );
        assert!(
            mapped_replacement_identifiers.contains(identifier),
            "traceability mapping rows must cite strengthened existing public proof {identifier:?}"
        );
    }
    for identifier in TRACEABILITY_RETIREMENT_ONLY_PROOF_IDENTIFIERS {
        assert!(
            traceability.contains(identifier),
            "traceability document must cite retirement-only proof {identifier:?}"
        );
        assert!(
            source_test_identifier_exists(identifier),
            "traceability retirement-only proof {identifier:?} has no source test"
        );
        assert!(
            mapped_replacement_identifiers.contains(identifier),
            "traceability mapping rows must cite retirement-only proof {identifier:?}"
        );
    }
}

#[test]
fn batch_thirteen_traceability_mapping_claim_is_exact() {
    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    let rows = traceability_mapping_rows(&traceability).expect("real mapping rows");
    let row = rows
        .into_iter()
        .find(|row| row.contains("native_open_protocol_default_drives_write_and_read"))
        .expect("Batch 13 traceability mapping row must exist");
    let (_, _, evidence) =
        traceability_mapping_cells(row).expect("Batch 13 traceability row must have three cells");
    assert_eq!(
        evidence,
        "**Compared Batch 13.** Historical protocol-only open default controls bare write/read; stripped framed arm match and bare `ping` CR addition; existing `AtPeer` proof remains stronger."
    );
}

#[test]
fn batch_fourteen_traceability_mapping_claim_is_exact() {
    const BATCH_FOURTEEN_CLAIM: &str =
        "**Compared Batch 14.** Static three JSON object response; explicit line framing plus `json_lines` parser; exact `140/0/0` timeout result with three ordered parsed objects and positions `52/192/0/0/0/192`; existing JSON Lines fixture proof remains stronger.";
    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    let rows = traceability_mapping_rows(&traceability).expect("real mapping rows");
    let row = rows
        .into_iter()
        .find(|row| row.contains("native_read_json_parser_decodes_jsonout"))
        .expect("Batch 14 traceability mapping row must exist");
    let (_, _, evidence) =
        traceability_mapping_cells(row).expect("Batch 14 traceability row must have three cells");
    assert_eq!(evidence, BATCH_FOURTEEN_CLAIM);

    let validation_rows: Vec<_> = traceability
        .lines()
        .filter(|line| {
            line.starts_with(
                "| `native_read_json_parser_decodes_jsonout` | three changing sensor objects |",
            )
        })
        .collect();
    assert_eq!(
        validation_rows.len(),
        1,
        "Batch 14 lower validation disposition row must exist exactly once"
    );
    let cells: Vec<_> = validation_rows[0]
        .trim()
        .split('|')
        .map(str::trim)
        .collect();
    assert_eq!(
        cells.len(),
        6,
        "Batch 14 lower validation disposition row must have four table cells"
    );
    assert_eq!(
        cells[4], BATCH_FOURTEEN_CLAIM,
        "Batch 14 implemented mapping and lower validation disposition must match"
    );
}

#[test]
fn batch_fifteen_traceability_mapping_claims_are_exact() {
    const CASE_A: &str = "native_read_ndjson_preset_decodes_json_frames";
    const CASE_A_CLAIM: &str =
        "**Compared Batch 15.** Static NDJSON payload `{\"a\":1}\\n\\n{\"b\":2}\\n`; `protocol: {\"type\":\"ndjson\"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `17/0/0` timeout result with ordered parsed `a`/`b` frames and positions `52/69/0/0/0/69`; stronger NDJSON fixture proof remains independent.";
    const CASE_B: &str = "native_read_ndjson_preset_skips_empty_lines";
    const CASE_B_CLAIM: &str =
        "**Compared Batch 15.** Static NDJSON payload `{\"a\":1}\\n\\n\\n{\"b\":2}\\n   \\n{\"c\":3}\\n`; `protocol: {\"type\":\"ndjson\"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `30/0/0` timeout result with ordered parsed `a`/`b`/`c` frames and positions `52/82/0/0/0/82`; blank and whitespace-only lines emit no frames; stronger NDJSON fixture proof remains independent.";

    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    let rows = traceability_mapping_rows(&traceability).expect("real mapping rows");
    for (native_case, claim, lower_prefix) in [
        (
            CASE_A,
            CASE_A_CLAIM,
            "| `native_read_ndjson_preset_decodes_json_frames` | two records plus blank |",
        ),
        (
            CASE_B,
            CASE_B_CLAIM,
            "| `native_read_ndjson_preset_skips_empty_lines` | records plus blank/whitespace lines |",
        ),
    ] {
        let row = rows
            .iter()
            .copied()
            .find(|row| row.contains(native_case))
            .unwrap_or_else(|| panic!("Batch 15 traceability row missing: {native_case}"));
        let (_, _, evidence) =
            traceability_mapping_cells(row).expect("Batch 15 mapping row must have three cells");
        assert_eq!(evidence, claim);

        let validation_rows: Vec<_> = traceability
            .lines()
            .filter(|line| line.starts_with(lower_prefix))
            .collect();
        assert_eq!(
            validation_rows.len(),
            1,
            "Batch 15 lower validation disposition row must exist exactly once for {native_case}"
        );
        let cells: Vec<_> = validation_rows[0]
            .trim()
            .split('|')
            .map(str::trim)
            .collect();
        assert_eq!(
            cells.len(),
            6,
            "Batch 15 lower validation disposition row must have four table cells"
        );
        assert_eq!(
            cells[4], claim,
            "Batch 15 mapping and lower validation disposition must match for {native_case}"
        );
    }
}

#[test]
fn native_traceability_mapping_guard_rejects_duplicate_and_unknown_rows() {
    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    let rows = traceability_mapping_rows(&traceability).expect("real mapping rows");
    let duplicate = format!("{}\n{}", rows.join("\n"), rows[0]);
    let duplicate_rows: Vec<_> = duplicate
        .lines()
        .filter(|line| line.starts_with("| `native_"))
        .collect();
    assert_ne!(
        duplicate_rows.len(),
        NATIVE_TRACEABILITY_ROWS.len(),
        "coverage lock must reject a duplicate native mapping row"
    );
    let unknown = rows[0].replacen(
        "native_ping_roundtrip",
        "native_unknown_traceability_case",
        1,
    );
    assert!(
        NATIVE_TRACEABILITY_ROWS
            .iter()
            .all(|native| !unknown.contains(native)),
        "coverage lock fixture must reject an unknown native mapping row"
    );
}

#[test]
fn line_framing_traceability_records_differential_payload_adaptation() {
    let traceability = repo_file("docs/development/native-sim-test-traceability.md");
    for marker in [
        "native_read_line_framing_splits_lines",
        "write cmd 1 ping",
        "info",
        "compile timestamp",
        "line_framing_returns_exact_ordered_peer_frames",
    ] {
        assert!(
            traceability.contains(marker),
            "line-framing traceability must retain {marker:?}"
        );
    }
    assert!(
        traceability.contains("not the original `pong`/`info`")
            && traceability.contains("does not compare")
            && traceability.contains("normalize that timestamp"),
        "line-framing traceability must disclose deterministic payload adaptation"
    );
}

const DIFFERENTIAL_BATCH_ONE_COMPARED_ROWS: [&str; 7] = [
    "native_ping_roundtrip",
    "native_split_writes_preserve_command_order",
    "native_get_status_after_write_increments_tx_counter",
    "native_reconfigure_baud_rate_persists",
    "native_named_connection_appears_in_list_connections",
    "native_set_flow_control_updates_summary_and_result",
    "native_open_with_flow_control_persists_in_summary",
];

const DIFFERENTIAL_BATCH_ONE_BASELINE_AND_STRONGER_ROWS: [&str; 1] =
    ["native_pending_read_then_write_ping_roundtrip"];
const DIFFERENTIAL_BATCH_ONE_BASELINE_PROOF: &str =
    "pending_read_receives_later_output_after_readiness_proven_hold";

const DIFFERENTIAL_BATCH_TWO_COMPARED_ROWS: [&str; 1] = ["native_read_line_framing_splits_lines"];
const DIFFERENTIAL_BATCH_TWO_BASELINE_AND_STRONGER_ROWS: [&str; 5] = [
    "native_read_regex_matches_pong",
    "native_read_glob_matches_pong_line",
    "native_read_framing_max_frames_stops",
    "native_read_framing_plus_match_combined",
    "native_explicit_rx_framing_beats_connection_default",
];
const DIFFERENTIAL_BATCH_TWO_BASELINE_PROOF_BINDINGS: [(&str, &str, &str); 5] = [
    (
        "native_read_regex_matches_pong",
        "regex_and_glob_matchers_find_complete_peer_line",
        "REGEX_GLOB_BASELINE_PROOFS",
    ),
    (
        "native_read_glob_matches_pong_line",
        "regex_and_glob_matchers_find_complete_peer_line",
        "REGEX_GLOB_BASELINE_PROOFS",
    ),
    (
        "native_read_framing_max_frames_stops",
        "max_frames_stops_after_exact_limit",
        "MAX_FRAMES_BASELINE_PROOFS",
    ),
    (
        "native_read_framing_plus_match_combined",
        "framing_plus_match_returns_matching_frame_and_index",
        "FRAMING_MATCH_BASELINE_PROOFS",
    ),
    (
        "native_explicit_rx_framing_beats_connection_default",
        "call_time_line_framing_beats_connection_delimiter_default",
        "OPEN_DEFAULT_BASELINE_PROOFS",
    ),
];
const DIFFERENTIAL_BATCH_THREE_COMPARED_ROWS: [&str; 3] = [
    "native_read_delimiter_framing_decodes",
    "native_read_start_end_framing_decodes",
    "native_read_explicit_line_endings_split_correctly",
];
const DIFFERENTIAL_BATCH_THREE_BASELINE_AND_STRONGER_ROWS: [&str; 2] = [
    "native_read_length_prefixed_framing_decodes",
    "native_write_tx_framing_modes_observed_via_trace",
];
const DIFFERENTIAL_BATCH_THREE_BASELINE_PROOF_BINDINGS: [(&str, &str, &str); 2] = [
    (
        "native_read_length_prefixed_framing_decodes",
        "delimiter_length_prefixed_and_start_end_decode_exact_payloads",
        "RAW_LENGTH_BASELINE_PROOFS",
    ),
    (
        "native_write_tx_framing_modes_observed_via_trace",
        "tx_framing_modes_produce_exact_independent_wire_vectors",
        "RAW_TX_BASELINE_PROOFS",
    ),
];
const DIFFERENTIAL_BATCH_FOUR_BASELINE_AND_STRONGER_ROWS: [&str; 2] = [
    "native_read_match_on_spam_complete",
    "native_read_buffer_budget_stops_under_flood",
];
const DIFFERENTIAL_BATCH_FOUR_BASELINE_PROOF_BINDINGS: [(&str, &str, &str); 2] = [
    (
        "native_read_match_on_spam_complete",
        "finite_flood_matcher_reaches_unique_completion_marker",
        "FLOOD_MATCHER_BASELINE_PROOFS",
    ),
    (
        "native_read_buffer_budget_stops_under_flood",
        "live_buffer_budget_caps_finite_flood_with_exact_stop_metadata",
        "FLOOD_BUFFER_BASELINE_PROOFS",
    ),
];
const DIFFERENTIAL_BATCH_FIVE_BASELINE_AND_STRONGER_ROWS: [&str; 3] = [
    "native_framing_reports_single_split_command",
    "native_trace_reports_exact_split_byte_sequence",
    "native_partial_line_buffered_then_completed",
];
const DIFFERENTIAL_BATCH_FIVE_BASELINE_PROOF_BINDINGS: [(&str, &str, &str); 3] = [
    (
        "native_framing_reports_single_split_command",
        "split_writes_preserve_one_command_and_exact_wire_order",
        "SPLIT_WRITES_BASELINE_PROOFS",
    ),
    (
        "native_trace_reports_exact_split_byte_sequence",
        "split_writes_preserve_one_command_and_exact_wire_order",
        "SPLIT_WRITES_BASELINE_PROOFS",
    ),
    (
        "native_partial_line_buffered_then_completed",
        "split_writes_preserve_one_command_and_exact_wire_order",
        "SPLIT_WRITES_BASELINE_PROOFS",
    ),
];
const DIFFERENTIAL_BATCH_NINE_BASELINE_AND_STRONGER_ROWS: [&str; 1] =
    ["native_read_slip_malformed_escape_returns_partial_result"];
const DIFFERENTIAL_BATCH_NINE_BASELINE_PROOF_BINDINGS: [(&str, &str, &str); 1] = [(
    "native_read_slip_malformed_escape_returns_partial_result",
    "slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery",
    "SLIP_MALFORMED_BASELINE_PROOFS",
)];
const DIFFERENTIAL_BATCH_TEN_COMPARED_ROWS: [&str; 1] =
    ["native_read_slip_recovers_after_error_on_next_call"];
const DIFFERENTIAL_BATCH_ELEVEN_COMPARED_ROWS: [&str; 1] =
    ["native_read_cobs_preset_decodes_frame"];
const DIFFERENTIAL_BATCH_TWELVE_COMPARED_ROWS: [&str; 1] = ["native_read_at_parser_parses_pong"];
const DIFFERENTIAL_BATCH_THIRTEEN_COMPARED_ROWS: [&str; 1] =
    ["native_open_protocol_default_drives_write_and_read"];
const DIFFERENTIAL_BATCH_FOURTEEN_COMPARED_ROWS: [&str; 1] =
    ["native_read_json_parser_decodes_jsonout"];
const DIFFERENTIAL_BATCH_FIFTEEN_COMPARED_ROWS: [&str; 2] = [
    "native_read_ndjson_preset_decodes_json_frames",
    "native_read_ndjson_preset_skips_empty_lines",
];
const DIFFERENTIAL_BATCH_SIX_COMPARED_ROWS: [&str; 1] =
    ["native_ack_command_provides_pre_execution_ack"];
const DIFFERENTIAL_BATCH_SEVEN_COMPARED_ROWS: [&str; 1] =
    ["native_flush_output_after_full_delivery_is_safe"];
const DIFFERENTIAL_BATCH_EIGHT_COMPARED_ROWS: [&str; 1] = ["native_read_slip_decodes_frame"];

fn differential_registry_body(source: &str) -> Result<&str, String> {
    let start = source
        .find("const REGISTRY: &[DifferentialRow] = &[")
        .ok_or_else(|| "native differential registry declaration is missing".to_string())?;
    let rest = &source[start..];
    let end = rest
        .find("\n];\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\npub struct RegistryCounts")
        .ok_or_else(|| "native differential registry declaration has no bounded end".to_string())?;
    Ok(&rest[..end])
}

fn differential_registry_row_status(body: &str, native_case: &str) -> Result<&'static str, String> {
    let marker = format!("\"{native_case}\"");
    let occurrence = body
        .find(&marker)
        .ok_or_else(|| format!("native differential registry lacks {native_case:?}"))?;
    let row_start = body[..occurrence]
        .rfind("DifferentialRow::")
        .ok_or_else(|| {
            format!("native differential registry row start missing for {native_case:?}")
        })?;
    let row = &body[row_start..occurrence];
    if row.starts_with("DifferentialRow::compared") {
        Ok("compared")
    } else if row.starts_with("DifferentialRow::baseline_and_stronger") {
        Ok("baseline_and_stronger")
    } else if row.starts_with("DifferentialRow::retired") {
        Ok("retired")
    } else if row.starts_with("DifferentialRow::pending") {
        Ok("pending")
    } else {
        Err(format!(
            "native differential registry row has unknown status for {native_case:?}: {row:?}"
        ))
    }
}

fn differential_registry_row_body<'a>(body: &'a str, native_case: &str) -> Result<&'a str, String> {
    let marker = format!("\"{native_case}\"");
    let occurrence = body
        .find(&marker)
        .ok_or_else(|| format!("native differential registry lacks {native_case:?}"))?;
    let row_start = body[..occurrence]
        .rfind("DifferentialRow::")
        .ok_or_else(|| {
            format!("native differential registry row start missing for {native_case:?}")
        })?;
    let row_end = body[occurrence..]
        .find("\n    DifferentialRow::")
        .map(|offset| occurrence + offset)
        .unwrap_or(body.len());
    Ok(&body[row_start..row_end])
}

#[test]
fn native_sim_differential_registry_and_docs_lock_batch_sets_and_counts() {
    let source = repo_file("tests/common/native_sim_differential/registry.rs");
    let body = differential_registry_body(&source)
        .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
    assert_eq!(
        body.matches("DifferentialRow::").count(),
        49,
        "differential registry must contain exactly 49 rows"
    );

    let mut compared = std::collections::BTreeSet::new();
    let mut baseline_and_stronger = std::collections::BTreeSet::new();
    let mut retired = std::collections::BTreeSet::new();
    let mut pending = 0usize;
    for native_case in NATIVE_TRACEABILITY_ROWS {
        let marker = format!("\"{native_case}\"");
        assert_eq!(
            body.matches(&marker).count(),
            1,
            "differential registry must represent {native_case:?} exactly once"
        );
        match differential_registry_row_status(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"))
        {
            "compared" => {
                compared.insert(native_case);
            }
            "baseline_and_stronger" => {
                baseline_and_stronger.insert(native_case);
            }
            "retired" => {
                retired.insert(native_case);
            }
            "pending" => pending += 1,
            status => panic!("unexpected parsed differential status {status:?}"),
        }
    }
    let expected_compared = DIFFERENTIAL_BATCH_ONE_COMPARED_ROWS
        .into_iter()
        .chain(DIFFERENTIAL_BATCH_TWO_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_THREE_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_SIX_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_SEVEN_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_EIGHT_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_TEN_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_ELEVEN_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_TWELVE_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_THIRTEEN_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_FOURTEEN_COMPARED_ROWS)
        .chain(DIFFERENTIAL_BATCH_FIFTEEN_COMPARED_ROWS)
        .collect();
    assert_eq!(
        compared, expected_compared,
        "differential registry compared-row set drifted"
    );
    assert_eq!(
        compared.len(),
        21,
        "differential registry compared-row count drifted"
    );
    let expected_baseline_and_stronger = DIFFERENTIAL_BATCH_ONE_BASELINE_AND_STRONGER_ROWS
        .into_iter()
        .chain(DIFFERENTIAL_BATCH_TWO_BASELINE_AND_STRONGER_ROWS)
        .chain(DIFFERENTIAL_BATCH_THREE_BASELINE_AND_STRONGER_ROWS)
        .chain(DIFFERENTIAL_BATCH_FOUR_BASELINE_AND_STRONGER_ROWS)
        .chain(DIFFERENTIAL_BATCH_FIVE_BASELINE_AND_STRONGER_ROWS)
        .chain(DIFFERENTIAL_BATCH_NINE_BASELINE_AND_STRONGER_ROWS)
        .collect();
    assert_eq!(
        baseline_and_stronger, expected_baseline_and_stronger,
        "differential registry baseline-and-stronger row set drifted"
    );
    assert_eq!(
        baseline_and_stronger.len(),
        14,
        "differential registry baseline-and-stronger count drifted"
    );
    assert_eq!(
        retired,
        RETIRED_NATIVE_TRACEABILITY_ROWS.into_iter().collect(),
        "differential registry retired-row set drifted"
    );
    assert_eq!(
        retired.len(),
        3,
        "differential registry retired-row count drifted"
    );
    assert_eq!(
        pending, 11,
        "differential registry pending-row count drifted"
    );

    for native_case in DIFFERENTIAL_BATCH_ONE_COMPARED_ROWS
        .into_iter()
        .chain(DIFFERENTIAL_BATCH_ONE_BASELINE_AND_STRONGER_ROWS)
    {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::CommandLifecycle"),
            "Batch 1 row {native_case:?} must retain explicit CommandLifecycle membership: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_TWO_COMPARED_ROWS
        .into_iter()
        .chain(DIFFERENTIAL_BATCH_TWO_BASELINE_AND_STRONGER_ROWS)
    {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::GenericMatchingFraming"),
            "Batch 2 row {native_case:?} must retain explicit GenericMatchingFraming membership: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_THREE_COMPARED_ROWS
        .into_iter()
        .chain(DIFFERENTIAL_BATCH_THREE_BASELINE_AND_STRONGER_ROWS)
    {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::RawGenericFraming"),
            "Batch 3 row {native_case:?} must retain explicit RawGenericFraming membership: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_FOUR_BASELINE_AND_STRONGER_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::FloodBuffer"),
            "Batch 4 row {native_case:?} must retain explicit FloodBuffer membership: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_FIVE_BASELINE_AND_STRONGER_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::CommandDiagnostics"),
            "Batch 5 row {native_case:?} must retain explicit CommandDiagnostics membership: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_SIX_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::AckState")
                && !row.contains("baseline_and_stronger")
                && !row.contains("SPLIT_WRITES_BASELINE_PROOFS"),
            "Batch 6 row {native_case:?} must be a direct AckState comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_SEVEN_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::OutputFlush")
                && row.contains("DifferentialCase::OutputFlushAfterDelivery")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 7 row {native_case:?} must be a direct OutputFlush comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_EIGHT_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::SlipHappy")
                && row.contains("DifferentialCase::SlipHappyPath")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 8 row {native_case:?} must be a direct SlipHappy comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_NINE_BASELINE_AND_STRONGER_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::SlipMalformed")
                && row.contains("DifferentialCase::SlipMalformedEscape")
                && row.starts_with("DifferentialRow::baseline_and_stronger")
                && row.contains("SLIP_MALFORMED_BASELINE_PROOFS"),
            "Batch 9 row {native_case:?} must be a baseline-and-stronger SlipMalformed comparison with its proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_TEN_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::SlipRecovery")
                && row.contains("DifferentialCase::SlipRecoveryAfterMalformed")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 10 row {native_case:?} must be a direct SlipRecovery comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_ELEVEN_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::CobsPreset")
                && row.contains("DifferentialCase::CobsPresetDecode")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 11 row {native_case:?} must be a direct CobsPreset comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_TWELVE_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::AtParser")
                && row.contains("DifferentialCase::AtParserPong")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 12 row {native_case:?} must be a direct AtParser comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_THIRTEEN_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::AtProtocolDefault")
                && row.contains("DifferentialCase::AtProtocolDefaultPong")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 13 row {native_case:?} must be a direct AtProtocolDefault comparison without a baseline proof binding: {row}"
        );
    }
    for native_case in DIFFERENTIAL_BATCH_FOURTEEN_COMPARED_ROWS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::JsonParser")
                && row.contains("DifferentialCase::JsonParserJsonout")
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 14 row {native_case:?} must be a direct JsonParser comparison without a baseline proof binding: {row}"
        );
    }
    for (native_case, case) in [
        (
            "native_read_ndjson_preset_decodes_json_frames",
            "DifferentialCase::NdjsonPresetJsonFrames",
        ),
        (
            "native_read_ndjson_preset_skips_empty_lines",
            "DifferentialCase::NdjsonPresetSkipsEmptyLines",
        ),
    ] {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            row.contains("DifferentialBatch::NdjsonPreset")
                && row.contains(case)
                && row.starts_with("DifferentialRow::compared")
                && !row.contains("baseline_and_stronger")
                && !row.contains("BASELINE_PROOFS"),
            "Batch 15 row {native_case:?} must be a direct NdjsonPreset comparison without a baseline proof binding: {row}"
        );
    }

    let batch_one_baseline =
        differential_registry_row_body(body, "native_pending_read_then_write_ping_roundtrip")
            .expect("Batch 1 baseline row must exist");
    assert!(
        batch_one_baseline.contains("PENDING_READ_BASELINE_PROOFS")
            && source.contains(DIFFERENTIAL_BATCH_ONE_BASELINE_PROOF),
        "differential registry Batch 1 baseline row must retain exact pending-read proof binding"
    );
    assert!(
        source_test_identifier_exists(DIFFERENTIAL_BATCH_ONE_BASELINE_PROOF),
        "differential registry Batch 1 baseline proof must retain a source test"
    );
    for (native_case, proof, proof_binding) in DIFFERENTIAL_BATCH_TWO_BASELINE_PROOF_BINDINGS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            source.contains(proof),
            "differential registry Batch 2 row {native_case:?} must retain proof {proof:?}"
        );
        assert!(
            source_test_identifier_exists(proof),
            "differential registry Batch 2 proof {proof:?} must retain a source test"
        );
        assert!(
            row.contains(proof_binding),
            "differential registry Batch 2 baseline row {native_case:?} must bind {proof_binding:?}: {row}"
        );
    }
    for (native_case, proof, proof_binding) in DIFFERENTIAL_BATCH_THREE_BASELINE_PROOF_BINDINGS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            source.contains(proof),
            "differential registry Batch 3 row {native_case:?} must retain proof {proof:?}"
        );
        assert!(
            source_test_identifier_exists(proof),
            "differential registry Batch 3 proof {proof:?} must retain a source test"
        );
        assert!(
            row.contains(proof_binding),
            "differential registry Batch 3 baseline row {native_case:?} must bind {proof_binding:?}: {row}"
        );
    }
    for (native_case, proof, proof_binding) in DIFFERENTIAL_BATCH_FOUR_BASELINE_PROOF_BINDINGS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            source.contains(proof),
            "differential registry Batch 4 row {native_case:?} must retain proof {proof:?}"
        );
        assert!(
            source_test_identifier_exists(proof),
            "differential registry Batch 4 proof {proof:?} must retain a source test"
        );
        assert!(
            row.contains(proof_binding),
            "differential registry Batch 4 baseline row {native_case:?} must bind {proof_binding:?}: {row}"
        );
    }
    for (native_case, proof, proof_binding) in DIFFERENTIAL_BATCH_FIVE_BASELINE_PROOF_BINDINGS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            source.contains(proof),
            "differential registry Batch 5 row {native_case:?} must retain proof {proof:?}"
        );
        assert!(
            source_test_identifier_exists(proof),
            "differential registry Batch 5 proof {proof:?} must retain a source test"
        );
        assert!(
            row.contains(proof_binding),
            "differential registry Batch 5 baseline row {native_case:?} must bind {proof_binding:?}: {row}"
        );
    }
    for (native_case, proof, proof_binding) in DIFFERENTIAL_BATCH_NINE_BASELINE_PROOF_BINDINGS {
        let row = differential_registry_row_body(body, native_case)
            .unwrap_or_else(|error| panic!("native differential registry invalid: {error}"));
        assert!(
            source.contains(proof),
            "differential registry Batch 9 row {native_case:?} must retain proof {proof:?}"
        );
        assert!(
            source_test_identifier_exists(proof),
            "differential registry Batch 9 proof {proof:?} must retain a source test"
        );
        assert!(
            row.contains(proof_binding),
            "differential registry Batch 9 baseline row {native_case:?} must bind {proof_binding:?}: {row}"
        );
    }

    for path in [
        "docs/development/native-sim-replacement-research-progress.md",
        "docs/development/native-sim-test-traceability.md",
        "docs/development/native-sim-replacement-recommendation.md",
    ] {
        let document = repo_file(path);
        for marker in [
            "17 compared",
            "18 compared",
            "19 compared",
            "15 compared",
            "16 compared",
            "14 compared",
            "13 baseline-and-stronger",
            "14 baseline-and-stronger",
            "3 retired",
            "17 pending",
            "15 pending",
            "14 pending",
            "13 pending",
            "16 pending",
            "19 pending",
            "18 pending",
            "16/14/3/16",
            "17/14/3/15",
            "18/14/3/14",
            "19/14/3/13",
            "21 compared",
            "11 pending",
            "21/14/3/11",
            "native_read_delimiter_framing_decodes",
            "native_read_length_prefixed_framing_decodes",
            "native_read_start_end_framing_decodes",
            "native_write_tx_framing_modes_observed_via_trace",
            "native_read_explicit_line_endings_split_correctly",
            "native_read_match_on_spam_complete",
            "native_read_buffer_budget_stops_under_flood",
            "Batch 4",
            "serial-mcp.native-sim-differential.flood-buffer-batch.v1",
            "flood-buffer-batch.json",
            "spam 1024 hex",
            "spam 512 hex",
            "max_buffered_bytes",
            "all six stable",
            "from_offset",
            "next_offset",
            "bytes_lost",
            "buffered_remaining",
            "start_offset",
            "end_offset",
            "elapsed_ms",
            "omitted",
            "modeled outcome",
            "request echoes",
            "prefilled",
            "excluded",
            "finite_flood_matcher_reaches_unique_completion_marker",
            "live_buffer_budget_caps_finite_flood_with_exact_stop_metadata",
            "native_framing_reports_single_split_command",
            "native_trace_reports_exact_split_byte_sequence",
            "native_partial_line_buffered_then_completed",
            "Batch 5",
            "serial-mcp.native-sim-differential.command-diagnostics-batch.v1",
            "command-diagnostics-batch.json",
            "LINE len=4 data=\"ping\"",
            "RX[0]=0x70",
            "split_writes_preserve_one_command_and_exact_wire_order",
            "Batch 6",
            "native_ack_command_provides_pre_execution_ack",
            "serial-mcp.native-sim-differential.ack-state-batch.v1",
            "ack-state-batch.json",
            "ack on\\r\\n",
            "ack 0\\r\\npong\\r\\n",
            "ack 1\\r\\npong\\r\\n",
            "ack 2\\r\\nack off\\r\\n",
            "pong\\r\\n",
            "32/40/0/0/0/40",
            "40/53/0/0/0/53",
            "53/66/0/0/0/66",
            "66/82/0/0/0/82",
            "82/88/0/0/0/88",
            "match_found",
            "no frames",
            "no drops",
            "no error",
            "no truncation",
            "ack_peer_orders_ack_before_response_and_stops_after_disable",
            "Batch 7",
            "native_flush_output_after_full_delivery_is_safe",
            "serial-mcp.native-sim-differential.output-flush-batch.v1",
            "output-flush-batch.json",
            "First matched `pong`",
            "output-only",
            "32/38/0/0/0/38",
            "38/44/0/0/0/44",
            "elapsed_ms",
            "intentional omission",
            "26 covered rows",
            "output_flush_after_full_delivery_preserves_later_traffic",
            "Batch 8",
            "native_read_slip_decodes_frame",
            "serial-mcp.native-sim-differential.slip-happy-batch.v1",
            "slip-happy-batch.json",
            "arm_cmd 1000",
            "arm_cmd delay=1000",
            "sendraw hex C0706F6E67C0",
            "c0 70 6f 6e 67 c0",
            "70 6f 6e 67",
            "max_frames",
            "52/58/0/0/0/58",
            "elapsed_ms",
            "27 covered rows",
            "slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery",
            "Batch 9",
            "native_read_slip_malformed_escape_returns_partial_result",
            "serial-mcp.native-sim-differential.slip-malformed-batch.v1",
            "slip-malformed-batch.json",
            "sendraw hex C0DB41C0",
            "c0 db 41 c0",
            "framing_error",
            "SLIP framing error: invalid escape byte 0x41",
            "52/56/0/0/0/56",
            "28 covered rows",
            "raw `rx_framing: slip`",
            "protocol: slip",
            "unmodeled request echoes",
            "Batch 10",
            "native_read_slip_recovers_after_error_on_next_call",
            "serial-mcp.native-sim-differential.slip-recovery-batch.v1",
            "slip-recovery-batch.json",
            "shared public double-arm/sendraw scaffold",
            "c0 db 41 c0",
            "c0 70 6f 6e 67 c0",
            "76/82/0/0/0/82",
            "bytes_read/bytes_observed/bytes_returned",
            "one raw SLIP frame",
            "29 covered rows",
            "Batch 11",
            "native_read_cobs_preset_decodes_frame",
            "serial-mcp.native-sim-differential.cobs-preset-batch.v1",
            "cobs-preset-batch.json",
            "static independent",
            "00 05 70 6f 6e 67 00",
            "sendraw hex 0005706F6E6700",
            "bytes_written=decoded_bytes=28",
            "protocol: {\"type\":\"cobs\"}",
            "raw COBS framing",
            "7/0/0",
            "timeout",
            "{\"parser\":\"raw\"}",
            "52/59/0/0/0/59",
            "broader zero-containing COBS TX/RX fixture proof",
            "30 covered rows",
            "Batch 12",
            "native_read_at_parser_parses_pong",
            "serial-mcp.native-sim-differential.at-parser-batch.v1",
            "at-parser-batch.json",
            "target/native-sim-differential/at-parser-characterization.json",
            "52/58/0/0/0/58",
            "6/0/0",
            "rx_framing: line",
            "rx_parser: at_command",
            "explicit-parser",
            "52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5",
            "31 covered rows",
            "Batch 13",
            "native_open_protocol_default_drives_write_and_read",
            "serial-mcp.native-sim-differential.at-protocol-default-batch.v1",
            "at-protocol-default-batch.json",
            "protocol-only",
            "bare `ping`",
            "4→5",
            "stripped framed arm match",
            "pong\\r\\n",
            "6/0/0",
            "52/58/0/0/0/58",
            "target/native-sim-differential/at-protocol-default-characterization.json",
            "cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775",
            "32 covered rows",
            "Batch 14",
            "native_read_json_parser_decodes_jsonout",
            "serial-mcp.native-sim-differential.json-parser-batch.v1",
            "json-parser-batch.json",
            "static three JSON object response",
            "explicit line",
            "json_lines",
            "140/0/0",
            "three ordered parsed objects",
            "52/192/0/0/0/192",
            "target/native-sim-differential/json-parser-characterization.json",
            "f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06",
            "existing stronger JSON Lines fixture proof",
            "stronger `AtPeer` proof",
            "Phase F blocked",
            "Batch 15",
            "native_read_ndjson_preset_decodes_json_frames",
            "native_read_ndjson_preset_skips_empty_lines",
            "serial-mcp.native-sim-differential.ndjson-preset-batch.v1",
            "ndjson-preset-batch.json",
            "sendraw hex 7B2261223A317D0A0A7B2262223A327D0A",
            "sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A",
            "protocol: {\"type\":\"ndjson\"}",
            "auto line",
            "skip_empty",
            "17/0/0",
            "30/0/0",
            "52/69/0/0/0/69",
            "52/82/0/0/0/82",
            "target/native-sim-differential/ndjson-characterization.json",
            "10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3",
            "ndjson_preset_parses_records_and_skips_blank_whitespace_lines",
            "35 covered rows",
        ] {
            assert!(
                document.contains(marker),
                "{path} must state native differential registry/document marker {marker:?}"
            );
        }
        let normalized = document.split_whitespace().collect::<Vec<_>>().join(" ");
        for stale in [
            "The independent 49-row registry now has 14 compared, 13 baseline-and-stronger",
            "The independent 49-row registry now has 14 compared, 14 baseline-and-stronger",
            "The independent registry is now 14/13/3/19",
            "The independent registry is now 14/14/3/18",
            "Registry status is now 14/13/3/19",
            "Registry status is now 14/14/3/18",
            "the 19 pending differential rows",
            "the 18 pending differential rows",
            "The registry is now 14/14/3/18",
        ] {
            assert!(
                !normalized.contains(stale),
                "{path} must not describe historical Batch 8 counts as current: {stale:?}"
            );
        }
        assert!(
            !normalized
                .contains("The Batch 6 checkpoint recorded 14 compared, 13 baseline-and-stronger"),
            "{path} must not label Batch 8 counts as a Batch 6 checkpoint"
        );
        assert!(
            !document.contains("full differential parity complete")
                && !document.contains("Phase F readiness proven"),
            "{path} must not claim differential batches provide full migration parity"
        );
    }
}

#[test]
fn phase_e_ci_and_xtask_wiring_stay_required() {
    let ci = workflow_file("ci.yml");
    let build_test = job_section(&ci, "build-test");
    let replacement_step = build_test
        .split("- name: Run required Rust PTY replacement suites")
        .nth(1)
        .and_then(|section| section.split("- name:").next())
        .expect("CI build-test job must contain required Rust PTY replacement step");
    for command in [
        "cargo test --locked --test device_fixture -- --test-threads=1",
        "cargo test --locked --test device_command_parity -- --test-threads=1",
        "cargo test --locked --test device_framing_parity -- --test-threads=1",
        "cargo test --locked --test device_protocol_parity -- --test-threads=1",
        "cargo test --locked --test device_parity_repeat phase_e_public_boundary_repeat_gate -- --ignored --test-threads=1",
    ] {
        assert!(
            replacement_step.contains(command),
            "required replacement CI step must execute {command:?}"
        );
    }
    assert!(
        replacement_step.contains("matrix.os == 'ubuntu-latest' || matrix.os == 'macos-14'"),
        "replacement CI step must run only Linux x86_64 and macOS arm64"
    );
    let xtask = repo_file("xtask/src/main.rs");
    for suite in [
        "device_fixture",
        "device_command_parity",
        "device_framing_parity",
        "device_protocol_parity",
    ] {
        assert!(
            xtask.contains(&format!("(\"{suite}\", false)")),
            "xtask test must run required replacement suite {suite:?} normally"
        );
    }
    let native = xtask
        .find("(\"native_sim_validation\", true)")
        .expect("xtask must retain native validation differential suite");
    let replacement = xtask
        .find("(\"device_fixture\", false)")
        .expect("xtask must run replacement fixture suite");
    assert!(
        replacement < native,
        "xtask must run required replacement suites before native differential suites"
    );
}

// ---------------------------------------------------------------------------
// GitHub Actions security regression guards (alerts #7–#14)
//
// The release pipeline must stay structural: trusted CI (push to main) calls
// the reusable release + registry workflows; nothing may reintroduce
// event-driven privileged orchestration (workflow_run / pull_request_target),
// secret inheritance, or event-derived checkouts.
// ---------------------------------------------------------------------------

/// Every committed workflow file in .github/workflows/, sorted by name.
/// Normalize workflow text to LF so parsing is independent of the checkout's
/// line endings (Windows clones are CRLF).
fn normalize_lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn workflow_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            let entry = entry.expect("readdir entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = fs::read_to_string(entry.path())
                .unwrap_or_else(|e| panic!("read {}: {e}", entry.path().display()));
            (name, normalize_lf(&text))
        })
        .filter(|(name, _)| name.ends_with(".yml") || name.ends_with(".yaml"))
        .collect();
    files.sort();
    files
}

fn workflow_file(name: &str) -> String {
    let (_, text) = workflow_files()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("missing workflow file {name:?}"));
    text
}

/// Everything in a workflow before its first column-0 `jobs:` key: the
/// `name` / `on` / `permissions` / `env` header.
fn workflow_header(wf: &str) -> &str {
    let jobs = wf
        .find("\r\njobs:")
        .or_else(|| wf.find("\njobs:"))
        .unwrap_or_else(|| panic!("workflow must declare a `jobs:` block"));
    &wf[..jobs]
}

/// The `on:` trigger block of a workflow: everything between the `on:` line
/// and the next column-0 (non-comment) key.
fn trigger_block(wf: &str) -> &str {
    let start = wf
        .find("\r\non:")
        .or_else(|| wf.find("\non:"))
        .unwrap_or_else(|| panic!("workflow must declare an `on:` trigger block"));
    // Skip past the "on:" line itself; the matched marker covers its line
    // ending (CRLF is 5 bytes, LF is 4).
    let skip = if wf[start..].starts_with("\r\n") {
        5
    } else {
        4
    };
    let rest = &wf[start + skip..];
    let mut end = rest.len();
    let mut pos = 0;
    for line in rest.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if !content.is_empty() && !content.starts_with(' ') && !content.starts_with('#') {
            end = pos;
            break;
        }
        pos += line.len();
    }
    &rest[..end]
}

/// One job block: from the `  <name>:` header through the line before the next
/// sibling job (a 2-space-indented `key:` line).
fn job_section<'a>(wf: &'a str, name: &str) -> &'a str {
    let marker = format!("\r\n  {name}:\r\n");
    let marker_lf = format!("\n  {name}:\n");
    let (start, marker_len) = match (wf.find(&marker), wf.find(&marker_lf)) {
        (Some(p), _) => (p, marker.len()),
        (None, Some(p)) => (p, marker_lf.len()),
        (None, None) => panic!("workflow must define a job named {name:?}"),
    };
    let rest = &wf[start + marker_len..];
    let mut end = rest.len();
    let mut pos = 0;
    for line in rest.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.starts_with("  ")
            && !content.starts_with("   ")
            && content.ends_with(':')
            && !content.starts_with("  #")
        {
            end = pos;
            break;
        }
        pos += line.len();
    }
    &rest[..end]
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The trusted-push gate every privileged caller job must carry.
fn has_trusted_push_gate(job: &str) -> bool {
    collapse_ws(job).contains("github.event_name == 'push' && github.ref == 'refs/heads/main'")
}

/// Forbidden privileged-orchestration patterns across a whole workflow file.
fn privileged_pattern_violations(wf: &str) -> Vec<&'static str> {
    [
        "workflow_run:",
        "pull_request_target:",
        "secrets: inherit",
        "github.event.workflow_run.head_sha",
    ]
    .into_iter()
    .filter(|p| wf.contains(p))
    .collect()
}

#[test]
fn workflows_never_use_privileged_triggers_or_event_content() {
    // No workflow may react to workflow_run / pull_request_target, inherit
    // secrets, or read workflow_run event content. This is the whole point of
    // the security remediation: privileged orchestration must not come back.
    for (name, text) in workflow_files() {
        let violations = privileged_pattern_violations(&text);
        assert!(
            violations.is_empty(),
            "{name} violates the privileged-orchestration contract: {violations:?}"
        );
    }
}

#[test]
fn privileged_pattern_guard_rejects_vulnerable_fixtures() {
    // Negative proof: each forbidden pattern must be detected, so the guard
    // cannot pass vacuously on a textually weakened workflow.
    let fixtures = [
        "on:\n  workflow_run:\n    types: [completed]\n",
        "on:\n  pull_request_target:\n    branches: [main]\n",
        "    secrets: inherit\n",
        "ref: ${{ github.event.workflow_run.head_sha }}\n",
    ];
    for fixture in fixtures {
        assert!(
            !privileged_pattern_violations(fixture).is_empty(),
            "privileged-pattern guard must reject fixture {fixture:?}"
        );
    }
}

/// Validate one workflow `uses:` line for immutable action pinning.
///
/// Local reusable-workflow references (`uses: ./.github/...`) are exempt.
/// Every external ref must be `owner/repo@<40-lowercase-hex-sha>` with a
/// trailing readable version comment (e.g. ` # v7`, ` # master`). Returns the
/// reason an offending line fails, or `None` when the line is compliant (or
/// not a `uses:` line). Extracted as a pure function so the full-file scan and
/// the synthetic negative proofs exercise the same code path.
fn external_action_pin_violation(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let value = trimmed.strip_prefix("uses:")?.trim();
    // Local reusable workflow references are exempt by design.
    if value.starts_with("./") {
        return None;
    }
    // Split off the trailing readable version comment, e.g. " # v7".
    let (ref_part, comment) = match value.split_once('#') {
        Some((r, c)) => (r.trim(), Some(c.trim())),
        None => (value, None),
    };
    let Some((action, revision)) = ref_part.rsplit_once('@') else {
        // A non-local ref without a valid `action@revision` shape (a bare
        // `actions/checkout`, a `docker://...` reference, ...) must fail —
        // it can never satisfy the immutable-SHA policy.
        return Some(format!(
            "external ref {ref_part:?} must be owner/repo@revision \
             (40-lowercase-hex SHA): {raw:?}"
        ));
    };
    let is_sha =
        revision.len() == 40 && revision.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    if !is_sha {
        return Some(format!(
            "external action {action} must be pinned to a 40-lowercase-hex SHA, \
             got {revision:?}: {raw:?}"
        ));
    }
    if comment.is_none() {
        return Some(format!(
            "external action {action} must carry a readable version comment \
             (e.g. ` # v7`): {raw:?}"
        ));
    }
    // The comment identifies the semantic version (`v7`, `v4.2.1`) or the
    // generic rust-toolchain action (`master`); anything else — including a
    // numeric prefix with trailing junk like `v7junk` — is not a version label.
    let comment = comment.unwrap();
    let comment_ok = comment == "master"
        || comment.strip_prefix('v').is_some_and(|rest| {
            !rest.is_empty()
                && rest.split('.').all(|component| {
                    !component.is_empty() && component.chars().all(|c| c.is_ascii_digit())
                })
        });
    if !comment_ok {
        return Some(format!(
            "external action {action} comment {comment:?} is not a readable \
             version label (expected `# v<N>` with numeric dot-separated \
             components, or `# master`): {raw:?}"
        ));
    }
    None
}

#[test]
fn workflow_external_actions_pinned_to_immutable_sha() {
    // Every external action ref in every workflow file must be an immutable
    // 40-lowercase-hex SHA (with a readable version comment) — never a mutable
    // `@vN` tag. Local `./` reusable workflow references are untouched.
    for (name, text) in workflow_files() {
        for (line_no, raw) in text.lines().enumerate() {
            if let Some(violation) = external_action_pin_violation(raw) {
                panic!("{name}:{} {violation}", line_no + 1);
            }
        }
    }
}

#[test]
fn workflow_action_pin_guard_rejects_mutable_tags() {
    // Negative proof: the guard must reject a synthetic mutable tag — the
    // exact regression this pinning pass removes — and accept pinned forms.
    let pinned = "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7";
    assert_eq!(external_action_pin_violation(pinned), None);
    assert_eq!(
        external_action_pin_violation(
            "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v4.2.1"
        ),
        None,
        "numeric dot-separated version comments are readable version labels"
    );
    assert_eq!(
        external_action_pin_violation("uses: ./.github/workflows/release.yml"),
        None,
        "local reusable workflow references stay exempt"
    );
    for mutable in [
        "uses: actions/checkout@v7",
        "uses: actions/checkout@main",
        "uses: actions/checkout",       // missing @revision entirely
        "uses: docker://alpine:latest", // docker ref without pinned digest policy
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", // missing comment
        "uses: actions/checkout@3D3C42E5AAC5BA805825DA76410C181273BA90B1 # v7", // uppercase
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # tag", // non-semver comment
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7junk", // junk after semver
    ] {
        assert!(
            external_action_pin_violation(mutable).is_some(),
            "pin guard must reject fixture {mutable:?}"
        );
    }
}

#[test]
fn ci_and_schema_drift_are_read_only_at_top_level() {
    for name in ["ci.yml", "schema-drift.yml"] {
        let text = workflow_file(name);
        let header = workflow_header(&text);
        assert!(
            header.contains("permissions:") && header.contains("contents: read"),
            "{name} must declare explicit top-level permissions: contents: read \
             (header: {header})"
        );
        assert!(
            !header.contains("contents: write"),
            "{name} top-level permissions must stay read-only"
        );
    }
}

#[test]
fn ci_release_caller_is_gated_ordered_and_narrow() {
    let ci = workflow_file("ci.yml");
    let release = job_section(&ci, "release");
    assert!(
        has_trusted_push_gate(release),
        "CI `release` caller job must be gated on a trusted push to main: {release}"
    );
    for required in ["nix-flake", "build-test", "native-sim"] {
        assert!(
            collapse_ws(release).contains(required),
            "CI `release` caller job must depend on every required CI job \
             ({required}): {release}"
        );
    }
    assert!(
        release.contains("uses: ./.github/workflows/release.yml"),
        "CI `release` caller job must call the local reusable release workflow"
    );
    assert!(
        release.contains("contents: write"),
        "CI `release` caller job must grant contents: write"
    );
    assert!(
        !release.contains("id-token"),
        "CI `release` caller job must not carry an OIDC token"
    );
    assert!(
        release.contains("ref: ${{ github.sha }}"),
        "CI `release` caller job must pass the immutable github.sha as ref"
    );
    assert!(
        release.contains("secrets:") && release.contains("CARGO_REGISTRY_TOKEN"),
        "CI `release` caller job must pass CARGO_REGISTRY_TOKEN explicitly: {release}"
    );
    assert!(
        !release.contains("secrets: inherit"),
        "CI `release` caller job must not inherit secrets"
    );
}

#[test]
fn ci_registry_caller_follows_release_read_only() {
    let ci = workflow_file("ci.yml");
    let registry = job_section(&ci, "publish-mcp-registry");
    assert!(
        has_trusted_push_gate(registry),
        "CI `publish-mcp-registry` caller job must be gated on a trusted push \
         to main: {registry}"
    );
    assert!(
        collapse_ws(registry).contains("needs: release"),
        "CI `publish-mcp-registry` caller job must run after the release job"
    );
    assert!(
        registry.contains("uses: ./.github/workflows/publish-mcp-registry.yml"),
        "CI `publish-mcp-registry` caller job must call the local reusable \
         registry workflow"
    );
    assert!(
        registry.contains("contents: read") && registry.contains("id-token: write"),
        "CI `publish-mcp-registry` caller job must grant only contents: read + \
         id-token: write"
    );
    assert!(
        !registry.contains("contents: write"),
        "CI `publish-mcp-registry` caller job must not grant contents: write"
    );
    assert!(
        !registry.contains("secrets:"),
        "CI `publish-mcp-registry` caller job must pass no secrets"
    );
    assert!(
        registry.contains("ref: ${{ github.sha }}"),
        "CI `publish-mcp-registry` caller job must pass the immutable \
         github.sha as ref"
    );
}

#[test]
fn trusted_push_gate_guard_rejects_removed_gate() {
    // Negative proof for the gate helper: a caller job without the exact
    // push+main condition must not satisfy has_trusted_push_gate.
    let ci = workflow_file("ci.yml");
    let release = job_section(&ci, "release");
    assert!(
        has_trusted_push_gate(release),
        "fixture must satisfy the gate"
    );
    let weakened = release.replace(
        "github.event_name == 'push' && github.ref == 'refs/heads/main'",
        "github.event_name == 'push'",
    );
    assert!(
        !has_trusted_push_gate(&weakened),
        "gate guard must reject a caller gated on push alone (any ref)"
    );
}

#[test]
fn release_and_registry_workflows_are_reusable_not_event_handlers() {
    for name in ["release.yml", "publish-mcp-registry.yml"] {
        let text = workflow_file(name);
        let triggers = trigger_block(&text);
        assert!(
            triggers.contains("workflow_call"),
            "{name} must be a reusable workflow (workflow_call), not an \
             independently privileged event handler; triggers: {triggers}"
        );
        for forbidden in [
            "workflow_run",
            "workflow_dispatch",
            "pull_request",
            "push:",
            "schedule",
        ] {
            assert!(
                !triggers.contains(forbidden),
                "{name} must not listen for the {forbidden} event — callers \
                 invoke it explicitly under their own event gates"
            );
        }
    }
}

#[test]
fn release_workflow_uses_job_level_least_privilege() {
    let release = workflow_file("release.yml");
    let header = workflow_header(&release);
    assert!(
        !header.contains("contents: write"),
        "release.yml must not declare broad workflow-level contents: write — \
         use job-level permissions"
    );
    let publish_crate = job_section(&release, "publish-crate");
    assert!(
        publish_crate.contains("contents: read"),
        "release.yml `publish-crate` job must be read-only (least privilege)"
    );
    assert!(
        !publish_crate.contains("contents: write"),
        "release.yml `publish-crate` job must not request contents: write"
    );
}

#[test]
fn release_code_executing_jobs_are_read_only() {
    // prepare, build, and publish-crate check out and execute repository code;
    // they must never hold repository write permission.
    let release = workflow_file("release.yml");
    for name in ["prepare", "build", "publish-crate"] {
        let job = job_section(&release, name);
        assert!(
            job.contains("contents: read"),
            "release.yml `{name}` job executes repository code and must be \
             contents: read"
        );
        assert!(
            !job.contains("contents: write"),
            "release.yml `{name}` job must not request contents: write"
        );
    }
}

#[test]
fn release_write_jobs_do_not_execute_project_code() {
    // The only write-permission jobs are the GitHub release mutators. They
    // must not check out or execute repository/project code.
    let release = workflow_file("release.yml");
    for name in ["create-draft", "publish-release"] {
        let job = job_section(&release, name);
        assert!(
            job.contains("contents: write"),
            "release.yml `{name}` job must hold the release write permission"
        );
        for forbidden in ["actions/checkout", "cargo ", "nix "] {
            assert!(
                !job.contains(forbidden),
                "release.yml `{name}` write job must not execute project code \
                 ({forbidden}): {job}"
            );
        }
    }
}

#[test]
fn release_later_checkouts_use_resolved_sha() {
    // inputs.ref is resolved once in prepare; every later checkout must pin
    // the immutable resolved SHA so a moving branch cannot swap content
    // between jobs.
    let release = workflow_file("release.yml");
    let prepare = job_section(&release, "prepare");
    assert!(
        prepare.contains("ref: ${{ inputs.ref }}"),
        "prepare must be the only job checking out inputs.ref"
    );
    for name in ["build", "publish-crate"] {
        let job = job_section(&release, name);
        assert!(
            job.contains("ref: ${{ needs.prepare.outputs.sha }}"),
            "release.yml `{name}` job must check out the resolved immutable SHA"
        );
        assert!(
            !job.contains("ref: ${{ inputs.ref }}"),
            "release.yml `{name}` job must not check out mutable inputs.ref"
        );
    }
}

#[test]
fn release_artifacts_flow_through_named_uploads_and_downloads() {
    // Build jobs upload deterministic per-platform artifacts; the publish job
    // downloads exactly those four and uploads them to the draft.
    let release = workflow_file("release.yml");
    let build = job_section(&release, "build");
    assert!(
        build.contains("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a")
            && build.contains("name: serial-mcp-${{ matrix.artifact_name_suffix }}"),
        "build job must upload deterministic named artifacts per platform"
    );
    let publish = job_section(&release, "publish-release");
    assert!(
        publish.contains("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"),
        "publish-release job must download the built artifacts"
    );
    for platform in [
        "x86_64-linux",
        "aarch64-linux",
        "aarch64-macos",
        "x86_64-windows",
    ] {
        assert!(
            publish.contains(&format!("name: serial-mcp-{platform}")),
            "publish-release job must download the {platform} artifact"
        );
    }
}

#[test]
fn release_dry_run_is_dispatch_only_read_only_and_never_publishes() {
    let dry = workflow_file("release-dry-run.yml");
    let triggers = trigger_block(&dry);
    assert!(
        triggers.contains("workflow_dispatch"),
        "release-dry-run.yml must be workflow_dispatch-only: {triggers}"
    );
    for forbidden in ["workflow_run", "pull_request", "push:", "schedule"] {
        assert!(
            !triggers.contains(forbidden),
            "release-dry-run.yml must not listen for the {forbidden} event"
        );
    }
    let header = workflow_header(&dry);
    assert!(
        header.contains("permissions:") && header.contains("contents: read"),
        "release-dry-run.yml must declare explicit read-only permissions"
    );
    assert!(
        !header.contains("contents: write"),
        "release-dry-run.yml must grant no write permission"
    );
    let job = job_section(&dry, "dry-run");
    assert!(
        job.contains("uses: ./.github/workflows/release.yml"),
        "release-dry-run.yml must call the local reusable release workflow"
    );
    assert!(
        collapse_ws(job).contains("mode: dry-run"),
        "release-dry-run.yml must hardcode mode: dry-run"
    );
    assert!(
        job.contains("ref: ${{ inputs.ref }}"),
        "release-dry-run.yml must forward the operator-selected ref"
    );
    assert!(
        !job.contains("secrets:") && !job.contains("CARGO_REGISTRY_TOKEN"),
        "release-dry-run.yml must pass no secrets"
    );
}

#[test]
fn workflow_parsers_accept_crlf_checkouts() {
    // Windows checkouts normalize line endings to CRLF; every parser must
    // yield identical results for CRLF and LF text (proven here, so a
    // future edit cannot silently break Windows CI).
    let release = workflow_file("release.yml");
    let crlf = release.replace('\n', "\r\n");
    assert_eq!(
        collapse_ws(workflow_header(&release)),
        collapse_ws(workflow_header(&crlf)),
        "workflow_header must parse CRLF text identically"
    );
    assert_eq!(
        collapse_ws(trigger_block(&release)),
        collapse_ws(trigger_block(&crlf)),
        "trigger_block must parse CRLF text identically"
    );
    for name in [
        "prepare",
        "build",
        "create-draft",
        "publish-release",
        "publish-crate",
    ] {
        assert_eq!(
            collapse_ws(job_section(&release, name)),
            collapse_ws(job_section(&crlf, name)),
            "job_section must parse the {name} job identically from CRLF text"
        );
    }
}

#[test]
fn flake_source_filter_ships_workflow_fixtures_and_scripts() {
    // The flake source filter must admit .github/workflows (doc_drift reads
    // them at runtime) and scripts/ (builder + unittest suite). A regression
    // here fails doc_drift under `nix flake check`; the flake's own
    // workflow-fixtures-present check is the executable proof.
    let flake = repo_file("flake.nix");
    let filter_start = flake
        .find("filter =")
        .expect("flake.nix must define a source filter");
    let filter = &flake[filter_start..filter_start + 3000];
    assert!(
        filter.contains("\"/.github\""),
        "flake source filter must include the .github directory tree (doc_drift reads it)"
    );
    assert!(
        filter.contains("\"/scripts\""),
        "flake source filter must include scripts/ (builder + unittest suite)"
    );
}

#[test]
fn publisher_workflow_uses_builder_and_no_nix_develop() {
    // The registry manifest must be built by the offline builder from
    // gh-downloaded assets; schema validation must use the independent
    // jsonschema-cli package, never `nix develop` (which realizes serial-mcp).
    let publisher = workflow_file("publish-mcp-registry.yml");
    let has_nix_develop_run = publisher.lines().any(|line| {
        // Ignore comment text; detect a "nix develop" invocation anywhere in
        // the command (not just at the start of the line).
        let code = line.split('#').next().unwrap_or("");
        code.split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["nix", "develop"])
    });
    assert!(
        !has_nix_develop_run,
        "publish-mcp-registry.yml must never run `nix develop` (it builds serial-mcp)"
    );
    for needle in [
        "scripts/build_registry_manifest.py",
        "gh release download",
        "git show",
        "nix shell",
        "jsonschema-cli",
        "gh api",
        "staging/server.json",
        "curl -fsSL",
        "--retry",
        "--json isDraft",
    ] {
        assert!(
            publisher.contains(needle),
            "publish-mcp-registry.yml must contain {needle:?}"
        );
    }
    for forbidden in ["--output server.json", "-i server.json", "curl -sL"] {
        assert!(
            !publisher.contains(forbidden),
            "publish-mcp-registry.yml must never {forbidden:?} (the repo-root \
             server.json template must stay untouched)"
        );
    }
}

#[test]
fn publisher_workflow_is_callable_with_ref_and_version() {
    let publisher = workflow_file("publish-mcp-registry.yml");
    let triggers = trigger_block(&publisher);
    assert!(
        triggers.contains("workflow_call"),
        "publisher must be a reusable workflow_call"
    );
    assert!(
        triggers.contains("ref:") && triggers.contains("version:"),
        "publisher must declare required ref and version inputs"
    );
    let header = workflow_header(&publisher);
    assert!(
        header.contains("contents: read") && header.contains("id-token: write"),
        "publisher must keep contents: read + id-token: write"
    );
    assert!(
        publisher.contains("ref: ${{ inputs.ref }}"),
        "publisher must check out the caller-supplied trusted ref"
    );
}

#[test]
fn publisher_consumes_staged_manifest_only() {
    // The generated manifest must live in staging and be consumed from there
    // by the builder, the schema validator, and the publisher. The committed
    // repo-root server.json template is never a write or read target of the
    // generated manifest.
    let publisher = workflow_file("publish-mcp-registry.yml");
    assert!(
        publisher.contains("--output staging/server.json"),
        "builder must write the generated manifest to staging/server.json"
    );
    assert!(
        publisher.contains("-i staging/server.json"),
        "schema validator must consume staging/server.json"
    );
    assert!(
        publisher.contains("publish staging/server.json"),
        "mcp-publisher must consume staging/server.json (pinned CLI 1.7.9 takes a positional path)"
    );
    assert!(
        !publisher.contains("--output server.json"),
        "builder must never write the repo-root server.json"
    );
    assert!(
        !publisher.contains("publish server.json"),
        "publisher must never consume the repo-root server.json"
    );
}

#[test]
fn publisher_rejects_unpublished_or_draft_releases() {
    // gh release view alone accepts drafts; the publisher must verify the
    // release is published (isDraft == false) via structured output.
    let publisher = workflow_file("publish-mcp-registry.yml");
    assert!(
        publisher.contains("--json isDraft") && publisher.contains(".isDraft"),
        "publisher must check isDraft via structured output"
    );
    assert!(
        !publisher.contains("gh release view \"$TAG\" >/dev/null"),
        "publisher must not rely on a bare gh release view success check"
    );
}

#[test]
fn publisher_backfill_is_dispatch_only_and_read_only() {
    let dry = workflow_file("publish-mcp-registry-backfill.yml");
    let triggers = trigger_block(&dry);
    assert!(
        triggers.contains("workflow_dispatch"),
        "backfill must be workflow_dispatch-only: {triggers}"
    );
    for forbidden in ["workflow_run", "pull_request", "push:", "schedule"] {
        assert!(
            !triggers.contains(forbidden),
            "backfill must not listen for the {forbidden} event"
        );
    }
    let header = workflow_header(&dry);
    assert!(
        header.contains("contents: read") && header.contains("id-token: write"),
        "backfill must be contents: read + id-token: write"
    );
    assert!(
        !header.contains("contents: write"),
        "backfill must grant no write permission"
    );
    let job = job_section(&dry, "backfill");
    assert!(
        job.contains("uses: ./.github/workflows/publish-mcp-registry.yml"),
        "backfill must call the local reusable publisher"
    );
    assert!(
        job.contains("ref: ${{ github.sha }}"),
        "backfill must pass the current trusted ref"
    );
    assert!(
        job.contains("version: ${{ inputs.version }}"),
        "backfill must forward the operator-supplied version"
    );
    assert!(
        !job.contains("secrets:") && !job.contains("actions/checkout"),
        "backfill must pass no secrets and never check out"
    );
}

#[test]
fn release_workflow_exposes_version_output() {
    let release = workflow_file("release.yml");
    let triggers = trigger_block(&release);
    assert!(
        triggers.contains("outputs:") && triggers.contains("jobs.prepare.outputs.version"),
        "release workflow_call must expose the prepared version as an output"
    );
    let prepare = job_section(&release, "prepare");
    assert!(
        prepare.contains("version: ${{ steps.version.outputs.value }}"),
        "prepare job must emit the version output"
    );
}

#[test]
fn ci_registry_caller_passes_release_version() {
    let ci = workflow_file("ci.yml");
    let registry = job_section(&ci, "publish-mcp-registry");
    assert!(
        registry.contains("version: ${{ needs.release.outputs.version }}"),
        "CI registry caller must pass the release-prepared version"
    );
    assert!(
        registry.contains("ref: ${{ github.sha }}"),
        "CI registry caller must pass the immutable github.sha"
    );
}

#[test]
fn ci_failure_notifier_is_push_main_and_narrow() {
    let ci = workflow_file("ci.yml");
    let notify = job_section(&ci, "notify-release-failure");
    assert!(
        collapse_ws(notify).contains("always()"),
        "notifier must run with always() so it fires when a dependency fails"
    );
    assert!(
        has_trusted_push_gate(notify),
        "notifier must be gated on trusted push to main"
    );
    assert!(
        collapse_ws(notify).contains("needs: [release, publish-mcp-registry]"),
        "notifier must depend on both the release and registry jobs"
    );
    assert!(
        notify.contains("issues: write"),
        "notifier must carry issues: write"
    );
    assert!(
        notify.contains("GH_REPO: ${{ github.repository }}"),
        "notifier has no checkout and must pin the repository via GH_REPO"
    );
    assert!(
        !notify.contains("head -n 1"),
        "notifier must not use a fragile head pipeline under pipefail"
    );
    for forbidden in [
        "contents: write",
        "id-token",
        "secrets:",
        "actions/checkout",
        "cargo ",
        "nix ",
    ] {
        assert!(
            !notify.contains(forbidden),
            "notifier must not carry {forbidden:?}"
        );
    }
}

fn collect_versions(v: &serde_json::Value, out: &mut Vec<String>) {
    if let serde_json::Value::Object(map) = v {
        for (k, val) in map {
            if k == "version" {
                if let serde_json::Value::String(s) = val {
                    out.push(s.clone());
                }
            }
            collect_versions(val, out);
        }
    } else if let serde_json::Value::Array(arr) = v {
        for val in arr {
            collect_versions(val, out);
        }
    }
}

/// Compare every `version` field in a committed registry template against the
/// Cargo package version. Returns each mismatching value; an empty vector
/// means aligned. Errors when the template is unreadable or carries no
/// version field at all (so the guard cannot pass vacuously on an empty tree).
fn server_json_version_mismatches(
    server_json: &str,
    cargo_version: &str,
) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(server_json)
        .map_err(|e| format!("server.json is not valid JSON: {e}"))?;
    // Collect every "version" field value anywhere in the JSON tree. With the
    // packages array stripped from the committed file this is currently just
    // the top-level field, but walking the whole tree keeps the guard honest
    // if versioned sections are ever added back.
    let mut versions: Vec<String> = Vec::new();
    collect_versions(&v, &mut versions);
    if versions.is_empty() {
        return Err("server.json must contain at least one \"version\" field — \
             did the schema change?"
            .to_string());
    }
    Ok(versions
        .into_iter()
        .filter(|ver| ver != cargo_version)
        .collect())
}

/// A minimal changelog satisfying the full contract for `version`. The
/// negative tests below mutate exactly one element so each rule fails for its
/// own named reason — this satisfies the plan's mutation-check requirement
/// without dirtying the real CHANGELOG.md during test runs.
fn synthetic_changelog(version: &str) -> String {
    let anchor = version.replace('.', "");
    format!(
        "# Changelog\n\
         \n\
         | Version | Date | Highlights |\n\
         | --- | --- | --- |\n\
         | [{version}](#{anchor}) | 2099-01-01 | synthetic entry |\n\
         \n\
         ## [Unreleased]\n\
         \n\
         - nothing yet\n\
         \n\
         ## [{version}]\n\
         \n\
         - released\n"
    )
}

/// Changelog contract for the current package version:
///
/// 1. release table contains a row beginning with `| [x.y.z](#xyz) |`
///    (anchor removes dots);
/// 2. body contains the exact heading `## [x.y.z]`;
/// 3. body contains the exact heading `## [Unreleased]`;
/// 4. the Unreleased heading occurs before the current release heading.
///
/// All matching is line-based and exact, so prose mentions never satisfy the
/// contract. Every violated rule is collected into one descriptive error, so
/// an earlier failure can never mask a later rule.
fn check_changelog_contract(changelog: &str, version: &str) -> Result<(), String> {
    let lines: Vec<&str> = changelog.lines().collect();
    let row_prefix = format!("| [{version}](#{}) |", version.replace('.', ""));
    let heading = format!("## [{version}]");
    let unreleased = "## [Unreleased]";

    let mut errors = Vec::new();
    if !lines.iter().any(|l| l.starts_with(&row_prefix)) {
        errors.push(format!(
            "CHANGELOG release table must contain a row starting with {row_prefix:?} \
             for version {version:?}"
        ));
    }
    if !lines.iter().any(|l| *l == heading) {
        errors.push(format!(
            "CHANGELOG body must contain the exact heading {heading:?}"
        ));
    }
    if !lines.contains(&unreleased) {
        errors.push(format!(
            "CHANGELOG body must contain the heading {unreleased:?}"
        ));
    }
    match (
        lines.iter().position(|l| *l == unreleased),
        lines.iter().position(|l| *l == heading),
    ) {
        (Some(u), Some(r)) if u >= r => errors.push(format!(
            "the {unreleased:?} heading must appear before the {heading:?} heading"
        )),
        _ => {}
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn cargo_toml_version() -> String {
    let manifest = repo_file("Cargo.toml");
    // Parse the [package] version = "..." line.
    let line = manifest
        .lines()
        .find(|l| l.starts_with("version = "))
        .expect("Cargo.toml must have a `version = \"...\"` line");
    // Extract the quoted string.
    let start = line.find('"').expect("version line has opening quote");
    let end = line.rfind('"').expect("version line has closing quote");
    line[start + 1..end].to_string()
}

#[test]
fn current_protocol_guide_omits_removed_streaming_tool() {
    // docs/protocols.md must describe only retained tools: no deleted
    // subscription notification types may appear in the current guide.
    // Historical docs (CHANGELOG, migration plans, baselines) are allowed
    // to mention old behavior; this guide is not.
    let guide = repo_file("docs/protocols.md");
    for needle in [
        "SubscribeStopNotification",
        "SubscribeEncodingErrorNotification",
        "subscribe",
    ] {
        assert!(
            !guide.contains(needle),
            "docs/protocols.md must not mention removed surface {needle:?}"
        );
    }
}

// =============================================================================
// Phase 4: pinned official conformance + Inspector gates
// =============================================================================

/// The exact pinned official conformance package (no floating tags).
const PINNED_CONFORMANCE_PACKAGE: &str = "@modelcontextprotocol/conformance@0.2.0-alpha.10";
/// The exact pinned official Inspector package (no floating tags).
const PINNED_INSPECTOR_PACKAGE: &str = "@modelcontextprotocol/inspector@2.0.0";
/// The exact direct version of the pinned conformance package in the
/// lockfile-pinned MCP validation npm project (compat/mcp-validation).
const PINNED_CONFORMANCE_VERSION: &str = "0.2.0-alpha.10";
/// The exact direct version of the pinned Inspector package in the
/// lockfile-pinned MCP validation npm project (compat/mcp-validation).
const PINNED_INSPECTOR_VERSION: &str = "2.0.0";
/// The exact pinned Node version for the conformance job.
const PINNED_NODE_VERSION: &str = "22.19.0";

/// The four documented fixture-dependent expected failures (server scope).
/// A baseline entry that starts passing must fail the run as stale; the
/// runner's exit-code contract enforces that, so these IDs must stay exact.
const EXPECTED_FAILURE_IDS: &[&str] = &[
    "server-stateless:sep-2575-server-rejects-undeclared-capability",
    "server-stateless:sep-2575-missing-capability-http-400",
    "server-stateless:sep-2575-http-server-no-independent-requests-on-stream",
    "server-stateless:sep-2575-server-no-log-without-loglevel",
];

/// The exact pinned historical rmcp 1.7.0 checksum (pre-migration resolution
/// of the current SDK's predecessor). A dependency bump that changes the
/// historical client implementation must fail here, not silently.
const RMCP_1_7_0_CHECKSUM: &str =
    "0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e";

/// The exact ordered `2025-11-25` official conformance scenario set. A new
/// legacy scenario must be added here and in the runner together.
const SCENARIOS_2025_11_25: &[&str] = &[
    "server-initialize",
    "ping",
    "completion-complete",
    "tools-list",
    "resources-list",
    "prompts-list",
];

/// The exact ordered `2026-07-28` official conformance scenario set. A new
/// modern scenario must be added here and in the runner together.
const SCENARIOS_2026_07_28: &[&str] = &[
    "server-stateless",
    "completion-complete",
    "tools-list",
    "resources-list",
    "prompts-list",
    "caching",
    "sep-2164-resource-not-found",
];

/// Extract the ordered word list from a quoted shell assignment
/// `VAR="word1 word2 ..."` in the compatibility runner. This is the exact
/// scenario parser: loose `contains` checks let a scenario drop or reorder
/// silently, the parsed array cannot.
fn parse_scenario_assignment(script: &str, var: &str) -> Vec<String> {
    let prefix = format!("{var}=\"");
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with(&prefix))
        .unwrap_or_else(|| panic!("runner script must define {var}=\"...\""));
    let open = line.find('"').expect("assignment has an opening quote");
    let rest = &line[open + 1..];
    let close = rest.find('"').expect("assignment has a closing quote");
    rest[..close]
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// The runner's exact per-version scenario contract: ordered scenario sets,
/// exact `--spec-version` values, and exact `-2025-11-25` / `-2026-07-28`
/// report suffixes. Returns on the first descriptive violation; each check
/// names the exact expected value so the drift stays diagnosable.
fn runner_scenario_contract(script: &str) -> Result<(), String> {
    let legacy = parse_scenario_assignment(script, "SCENARIOS_2025_11_25");
    if legacy != SCENARIOS_2025_11_25 {
        return Err(format!(
            "2025-11-25 scenario set must be exactly {SCENARIOS_2025_11_25:?}, \
             parsed {legacy:?}"
        ));
    }
    let modern = parse_scenario_assignment(script, "SCENARIOS_2026_07_28");
    if modern != SCENARIOS_2026_07_28 {
        return Err(format!(
            "2026-07-28 scenario set must be exactly {SCENARIOS_2026_07_28:?}, \
             parsed {modern:?}"
        ));
    }
    for (version, suffix) in [("2025-11-25", "-2025-11-25"), ("2026-07-28", "-2026-07-28")] {
        if !script.contains(&format!("--spec-version {version}")) {
            return Err(format!(
                "runner must run conformance at the exact --spec-version {version}"
            ));
        }
        if !script.contains(&format!("\"$REPORT_DIR/$sc{suffix}\"")) {
            return Err(format!(
                "runner must write report dir \"$REPORT_DIR/$sc{suffix}\""
            ));
        }
    }
    Ok(())
}

#[test]
fn conformance_expected_failures_are_exactly_the_four_documented_checks() {
    let file = repo_file("conformance/expected-failures.yaml");
    let server_section = file
        .split("server:")
        .nth(1)
        .expect("expected-failures.yaml must have a server: list");
    for id in EXPECTED_FAILURE_IDS {
        assert!(
            server_section.contains(&format!("  - {id}")),
            "expected-failures.yaml must baseline exactly {id:?}"
        );
    }
    // No other baselined checks: every list line under server: is one of the
    // four documented IDs.
    let baselined: Vec<&str> = server_section
        .lines()
        .filter(|l| l.trim_start().starts_with("- "))
        .map(|l| l.trim())
        .collect();
    assert_eq!(
        baselined.len(),
        EXPECTED_FAILURE_IDS.len(),
        "expected-failures.yaml must baseline exactly {} checks, got {baselined:?}",
        EXPECTED_FAILURE_IDS.len()
    );
}

#[test]
fn ci_conformance_job_pins_packages_and_never_runs_suite_all() {
    let ci = repo_file(".github/workflows/ci.yml");
    let job = ci
        .split("mcp-conformance:")
        .nth(1)
        .expect("ci.yml must define the mcp-conformance job");
    // The job delegates compatibility execution to the shared runner: local
    // and CI must share one executable path. Exact package pins, scenario
    // lists, and the expected-failure baseline live in the runner script
    // (guarded by `compat_runner_pins_packages_and_never_runs_suite_all` and
    // `ci_scenario_lists_match_pinned_runner_scenarios`).
    assert!(
        job.contains("bash scripts/test-mcp-compat.sh"),
        "mcp-conformance job must invoke the shared runner"
    );
    // The job delegates ALL compatibility execution to the runner: it must
    // not duplicate scenario loops (no --scenario/--spec-version invocations
    // may appear in CI YAML).
    let scenario_loops: Vec<&str> = job
        .lines()
        .filter(|l| l.contains("--scenario") && !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        scenario_loops.is_empty(),
        "mcp-conformance job must not duplicate scenario loops (they live in \
         scripts/test-mcp-compat.sh): {scenario_loops:?}"
    );
    assert!(
        job.contains(PINNED_NODE_VERSION),
        "mcp-conformance job must pin Node {PINNED_NODE_VERSION}"
    );
    assert!(
        job.contains("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"),
        "mcp-conformance job must upload reports with pinned actions/upload-artifact"
    );
    assert!(
        job.contains("retention-days: 7"),
        "mcp-conformance job must keep reports for 7 days"
    );
    assert!(
        job.contains("timeout-minutes: 15"),
        "mcp-conformance job must be bounded to 15 minutes"
    );
    assert!(
        job.contains("permissions:") && job.contains("contents: read"),
        "mcp-conformance job must run with contents: read permissions"
    );
    // Comments may explain why `--suite all` is forbidden; only an actual
    // (non-comment) usage counts.
    let suite_all_usage: Vec<&str> = job
        .lines()
        .filter(|l| l.contains("--suite all") && !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        suite_all_usage.is_empty(),
        "mcp-conformance job must never run `--suite all`: {suite_all_usage:?}"
    );
    // Validation is fully lockfile-pinned: the CI job must never resolve
    // packages dynamically with npx (the runner installs from the committed
    // lockfile with lifecycle scripts disabled). Comments may explain the
    // rule; only actual usage counts.
    let npx_usage: Vec<&str> = job
        .lines()
        .filter(|l| l.contains("npx") && !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        npx_usage.is_empty(),
        "mcp-conformance job must never use npx (validation is lockfile-pinned): {npx_usage:?}"
    );
}

#[test]
fn compat_runner_pins_packages_and_never_runs_suite_all() {
    // The shared runner is the executable compatibility gate: it must install
    // the lockfile-pinned validation tree with `npm ci --ignore-scripts`
    // (lifecycle scripts disabled, no npx), invoke the local conformance /
    // mcp-inspector binaries directly, wire the Inspector smoke script, apply
    // the exact expected-failure baseline path, exercise the historical
    // fixture over BOTH transports, run under `set -euo pipefail`, and never
    // run `--suite all` (comments may explain why it is forbidden).
    let script = repo_file("scripts/test-mcp-compat.sh");
    assert!(
        script.contains(PINNED_CONFORMANCE_PACKAGE),
        "test-mcp-compat.sh must document the pinned {PINNED_CONFORMANCE_PACKAGE}"
    );
    assert!(
        script.contains("npm ci --ignore-scripts"),
        "test-mcp-compat.sh must install the lockfile-pinned validation tree \
         with npm ci --ignore-scripts (lifecycle scripts disabled)"
    );
    assert!(
        script.contains("node_modules/.bin/conformance"),
        "test-mcp-compat.sh must invoke the local locked conformance binary \
         (compat/mcp-validation/node_modules/.bin/conformance)"
    );
    assert!(
        script.contains("node_modules/.bin/mcp-inspector"),
        "test-mcp-compat.sh must invoke the local locked mcp-inspector binary \
         (compat/mcp-validation/node_modules/.bin/mcp-inspector)"
    );
    assert!(
        script.contains("EXPECTED_FAILURES=\"$ROOT/conformance/expected-failures.yaml\""),
        "test-mcp-compat.sh must point EXPECTED_FAILURES at the exact baseline path"
    );
    assert!(
        script.contains("--expected-failures \"$EXPECTED_FAILURES\""),
        "test-mcp-compat.sh must apply the exact expected-failures baseline"
    );
    assert!(
        script.contains("\"$FIXTURE_BIN\" stdio \"$BIN\""),
        "test-mcp-compat.sh must exercise the historical fixture over stdio"
    );
    assert!(
        script.contains("\"$FIXTURE_BIN\" http \"$MCP_URL\""),
        "test-mcp-compat.sh must exercise the historical fixture over HTTP"
    );
    assert!(
        script.contains("node ") && script.contains("inspector-smoke.mjs"),
        "test-mcp-compat.sh must run the Inspector smoke script via node"
    );
    assert!(
        script.contains("set -euo pipefail"),
        "test-mcp-compat.sh must fail hard under set -euo pipefail"
    );
    let suite_all_usage: Vec<&str> = script
        .lines()
        .filter(|l| l.contains("--suite all") && !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        suite_all_usage.is_empty(),
        "test-mcp-compat.sh must never run `--suite all`: {suite_all_usage:?}"
    );
    let npx_usage: Vec<&str> = script
        .lines()
        .filter(|l| l.contains("npx") && !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        npx_usage.is_empty(),
        "test-mcp-compat.sh must never resolve packages via npx: {npx_usage:?}"
    );
}

#[test]
fn compat_runner_scenario_contract_is_exact_per_version() {
    // The exact version-indexed scenario contract: ordered quoted
    // assignments, exact --spec-version values, and exact report suffixes.
    runner_scenario_contract(&repo_file("scripts/test-mcp-compat.sh"))
        .unwrap_or_else(|e| panic!("runner scenario contract violated: {e}"));
}

/// The committed MCP validation npm project contract: a private package.json
/// with EXACT direct dependency versions (no `^`/`~`/tags/ranges), and a
/// committed package-lock.json whose lockfile-root dependencies are the exact
/// same versions and whose locked conformance + Inspector package entries
/// carry exact versions plus `sha512-` integrity hashes. The validation flow
/// installs ONLY from this lockfile (`npm ci --ignore-scripts`) — a lockfile
/// that no longer resolves the pinned versions or loses integrity breaks the
/// supply-chain pin.
///
/// The `overrides` block is a peer-range fix, not a product dependency: the
/// Inspector's transitive `ink-form@2.0.1` pulls `ink-select-input@5.0.0`
/// (peer `ink ^4`) alongside `ink-text-input@6.0.0` (peer `ink >=5`), which
/// no single ink instance can satisfy — npm auto-overrides that peer conflict
/// into an INVALID tree (`npm ls` exits nonzero). Pinning `ink` to `6.8.0`
/// and `ink-form`'s `ink-select-input` to `6.2.0` (peer `ink >=5.0.0`) makes
/// every ink peer edge satisfiable by one instance, so `npm ls --all` is
/// clean. Both pins are exact versions, so they stay fully lockfile-pinned.
fn mcp_validation_npm_contract(manifest: &str, lock: &str) -> Result<(), String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest)
        .map_err(|e| format!("package.json must be valid JSON: {e}"))?;
    let manifest = manifest
        .as_object()
        .ok_or("package.json must be a JSON object")?;
    if manifest.get("private").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("package.json must be private".to_string());
    }
    let deps = manifest
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
        .ok_or("package.json must carry a dependencies object")?;
    let expected = [
        (
            "@modelcontextprotocol/conformance",
            PINNED_CONFORMANCE_VERSION,
        ),
        ("@modelcontextprotocol/inspector", PINNED_INSPECTOR_VERSION),
    ];
    if deps.len() != expected.len() {
        return Err(format!(
            "package.json must pin exactly {} direct dependencies, got {}",
            expected.len(),
            deps.len()
        ));
    }
    for (name, version) in expected {
        match deps.get(name).and_then(serde_json::Value::as_str) {
            Some(v) if v == version => {}
            other => {
                return Err(format!(
                    "package.json must pin {name} at exactly {version}, got {other:?}"
                ));
            }
        }
    }

    // Peer-range fix guard: without these exact overrides npm resolves the
    // ink-form peer conflict into an invalid tree (`npm ls` fails). If the
    // upstream graph is ever fixed so the overrides become removable, update
    // this guard together with the overrides and re-run `npm ls --all` in
    // compat/mcp-validation.
    let overrides = manifest
        .get("overrides")
        .and_then(serde_json::Value::as_object)
        .ok_or("package.json must carry the ink peer-range overrides")?;
    match overrides.get("ink").and_then(serde_json::Value::as_str) {
        Some("6.8.0") => {}
        other => {
            return Err(format!(
                "package.json must override ink to exactly 6.8.0, got {other:?}"
            ));
        }
    }
    let ink_form = overrides
        .get("ink-form")
        .and_then(serde_json::Value::as_object)
        .ok_or("package.json must override ink-form's ink-select-input")?;
    match ink_form
        .get("ink-select-input")
        .and_then(serde_json::Value::as_str)
    {
        Some("6.2.0") => {}
        other => {
            return Err(format!(
                "package.json must override ink-form's ink-select-input to \
                 exactly 6.2.0, got {other:?}"
            ));
        }
    }

    let lock: serde_json::Value = serde_json::from_str(lock)
        .map_err(|e| format!("package-lock.json must be valid JSON: {e}"))?;
    let lock = lock
        .as_object()
        .ok_or("package-lock.json must be a JSON object")?;
    let packages = lock
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .ok_or("package-lock.json must carry a packages map (lockfileVersion 3)")?;
    let root = packages
        .get("")
        .and_then(serde_json::Value::as_object)
        .ok_or("package-lock.json must carry a root packages entry")?;
    let root_deps = root
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
        .ok_or("lockfile root must carry exact dependencies")?;
    if root_deps.len() != expected.len() {
        return Err(format!(
            "lockfile root must pin exactly {} direct dependencies, got {}",
            expected.len(),
            root_deps.len()
        ));
    }
    for (name, version) in expected {
        match root_deps.get(name).and_then(serde_json::Value::as_str) {
            Some(v) if v == version => {}
            other => {
                return Err(format!(
                    "lockfile root must resolve {name} at exactly {version}, got {other:?}"
                ));
            }
        }
        let entry = packages
            .get(&format!("node_modules/{name}"))
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("lockfile must carry a locked node_modules/{name} entry"))?;
        if entry.get("version").and_then(serde_json::Value::as_str) != Some(version) {
            return Err(format!(
                "locked {name} entry must resolve exactly {version}, got {:?}",
                entry.get("version").and_then(serde_json::Value::as_str)
            ));
        }
        let integrity = entry.get("integrity").and_then(serde_json::Value::as_str);
        match integrity {
            Some(i) if i.starts_with("sha512-") && !i.is_empty() => {}
            other => {
                return Err(format!(
                    "locked {name} entry must carry a sha512- integrity hash, got {other:?}"
                ));
            }
        }
    }

    // The overridden ink peer range must resolve to the exact overridden
    // versions in the lockfile too (locked `ink` 6.8.0 + `ink-select-input`
    // 6.2.0), so a lockfile regenerated without the overrides fails here.
    for (locked_name, version) in [("ink", "6.8.0"), ("ink-select-input", "6.2.0")] {
        let entry = packages
            .get(&format!("node_modules/{locked_name}"))
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                format!("lockfile must carry a locked node_modules/{locked_name} entry")
            })?;
        if entry.get("version").and_then(serde_json::Value::as_str) != Some(version) {
            return Err(format!(
                "locked {locked_name} entry must resolve exactly {version}, got {:?}",
                entry.get("version").and_then(serde_json::Value::as_str)
            ));
        }
    }
    Ok(())
}

#[test]
fn mcp_validation_npm_tree_is_lockfile_pinned() {
    // Observable supply-chain contract: exact direct versions, exact
    // lockfile-root deps, and locked per-package versions + integrity for the
    // conformance and Inspector packages. The runner installs from this
    // lockfile with lifecycle scripts disabled and never via npx.
    mcp_validation_npm_contract(
        &repo_file("compat/mcp-validation/package.json"),
        &repo_file("compat/mcp-validation/package-lock.json"),
    )
    .unwrap_or_else(|e| panic!("MCP validation npm contract violated: {e}"));
}

#[test]
fn mcp_validation_contract_rejects_a_drifted_direct_version() {
    // Negative proof: drifting the manifest's conformance direct version away
    // from the lockfile must fail the contract naming the package.
    let manifest = repo_file("compat/mcp-validation/package.json");
    let mutated = manifest.replace(
        &format!("\"@modelcontextprotocol/conformance\": \"{PINNED_CONFORMANCE_VERSION}\""),
        "\"@modelcontextprotocol/conformance\": \"0.2.0-alpha.9\"",
    );
    assert_ne!(mutated, manifest, "mutation must change the manifest");
    let err = mcp_validation_npm_contract(
        &mutated,
        &repo_file("compat/mcp-validation/package-lock.json"),
    )
    .unwrap_err();
    assert!(
        err.contains("conformance"),
        "failure must name the drifted package: {err}"
    );
}

#[test]
fn scenario_contract_rejects_a_dropped_scenario_word() {
    // Negative proof for the exact-scenario parser/check: removing one word
    // from the real 2025-11-25 assignment must fail the contract naming the
    // scenario set (the loose `contains` checks it replaces could not catch
    // this).
    let script = repo_file("scripts/test-mcp-compat.sh");
    let assignment = script
        .lines()
        .find(|l| l.starts_with("SCENARIOS_2025_11_25="))
        .expect("runner defines the 2025-11-25 assignment");
    let mutated = script.replace(assignment, &assignment.replace(" ping", ""));
    let err = runner_scenario_contract(&mutated).unwrap_err();
    assert!(
        err.contains("2025-11-25 scenario set"),
        "failure must name the scenario set: {err}"
    );
}

#[test]
fn scenario_contract_rejects_a_drifted_report_suffix() {
    // Negative proof: drifting the modern report suffix must fail the
    // contract naming the report dir.
    let script = repo_file("scripts/test-mcp-compat.sh");
    let mutated = script.replace(
        "\"$REPORT_DIR/$sc-2026-07-28\"",
        "\"$REPORT_DIR/$sc-2026-07-29\"",
    );
    let err = runner_scenario_contract(&mutated).unwrap_err();
    assert!(
        err.contains("report dir"),
        "failure must name the report dir rule: {err}"
    );
}

#[test]
fn historical_fixture_pins_exact_rmcp_1_7_0() {
    // The fixture is the real historical-client proof: its manifest must pin
    // rmcp exactly =1.7.0 with default-features = false and only the required
    // client/transport features, and its committed lockfile must resolve
    // exactly one rmcp package at 1.7.0 with the historical checksum. A
    // bumped or loosened dependency silently changes the client
    // implementation under test.
    let manifest: toml::Value = toml::from_str(&repo_file("compat/rmcp-1-client/Cargo.toml"))
        .expect("compat/rmcp-1-client/Cargo.toml must be valid TOML");
    let rmcp = manifest
        .get("dependencies")
        .and_then(|d| d.get("rmcp"))
        .expect("fixture manifest must depend on rmcp");
    let table = rmcp.as_table().expect("rmcp dependency must be a table");
    assert_eq!(
        table.get("version").and_then(toml::Value::as_str),
        Some("=1.7.0"),
        "fixture must pin rmcp exactly =1.7.0"
    );
    assert_eq!(
        table.get("default-features").and_then(toml::Value::as_bool),
        Some(false),
        "fixture must use default-features = false"
    );
    let features: Vec<&str> = table
        .get("features")
        .and_then(toml::Value::as_array)
        .expect("rmcp dependency must declare features")
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    for feature in [
        "client",
        "transport-child-process",
        "transport-streamable-http-client-reqwest",
    ] {
        assert!(
            features.contains(&feature),
            "fixture must keep the rmcp {feature:?} feature, got {features:?}"
        );
    }
    assert_eq!(
        features.len(),
        3,
        "fixture must declare exactly the three required rmcp features: {features:?}"
    );

    let lock: toml::Value = toml::from_str(&repo_file("compat/rmcp-1-client/Cargo.lock"))
        .expect("compat/rmcp-1-client/Cargo.lock must be valid TOML");
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .expect("Cargo.lock must carry a [[package]] array");
    let rmcp_entries: Vec<&toml::Value> = packages
        .iter()
        .filter(|p| p.get("name").and_then(toml::Value::as_str) == Some("rmcp"))
        .collect();
    assert_eq!(
        rmcp_entries.len(),
        1,
        "fixture lockfile must contain exactly one rmcp package entry, got {}",
        rmcp_entries.len()
    );
    let entry = rmcp_entries[0];
    assert_eq!(
        entry.get("version").and_then(toml::Value::as_str),
        Some("1.7.0"),
        "fixture lockfile must resolve rmcp at exactly 1.7.0"
    );
    assert_eq!(
        entry.get("checksum").and_then(toml::Value::as_str),
        Some(RMCP_1_7_0_CHECKSUM),
        "fixture lockfile must resolve the historical rmcp checksum"
    );
}

#[test]
fn policy_doc_states_support_table_and_permanent_legacy_contract() {
    // The durable compatibility policy must name both versions in preferred
    // order (2026-07-28 first), the permanent 2025-11-25 retention rule, the
    // exact shared runner command, the historical fixture, and the
    // no-implicit-known-version-support rule. Anchor checks only — no brittle
    // whole-prose snapshot.
    let policy = repo_file("docs/development/mcp-version-compatibility-policy.md");
    let modern = policy
        .find("2026-07-28")
        .expect("policy doc must name 2026-07-28");
    let legacy = policy
        .find("2025-11-25")
        .expect("policy doc must name 2025-11-25");
    assert!(
        modern < legacy,
        "policy doc must list 2026-07-28 before 2025-11-25 (preferred first)"
    );
    assert!(
        policy.contains("permanent") && policy.contains("2025-11-25"),
        "policy doc must state the permanent 2025-11-25 retention rule"
    );
    assert!(
        policy.contains("bash scripts/test-mcp-compat.sh"),
        "policy doc must document the exact shared runner command"
    );
    assert!(
        policy.contains("compat/rmcp-1-client"),
        "policy doc must name the historical rmcp 1.7.0 fixture"
    );
    assert!(
        policy.contains("KNOWN_VERSIONS"),
        "policy doc must state that rmcp known versions do not imply support"
    );
    assert!(
        policy.contains("never inferred"),
        "policy doc must state support is never inferred (date ordering etc.)"
    );
}

#[test]
fn features_md_tracks_pre_2025_11_25_as_demand_driven_feature_idea() {
    // Older protocol revisions are a potential feature, not current support:
    // FEATURES.md must carry the item under Wish, label it non-current and
    // demand-driven, and keep the supported set at exactly the two versions.
    let features = repo_file("docs/development/FEATURES.md");
    assert!(
        features.contains("Earlier MCP protocol revisions (pre-2025-11-25)"),
        "FEATURES.md must track the pre-2025-11-25 item under Wish"
    );
    assert!(
        features.contains("NOT current support"),
        "FEATURES.md must state the item is not current support"
    );
    assert!(
        features.contains("demand"),
        "FEATURES.md must label the item as demand-driven"
    );
    assert!(
        features.contains("2026-07-28") && features.contains("2025-11-25"),
        "FEATURES.md must state the supported set remains exactly the two versions"
    );
}

#[test]
fn ci_scenario_lists_match_pinned_runner_scenarios() {
    // The pinned conformance package provides no `server-session-lifecycle`
    // scenario; the legacy initialize/session lifecycle is covered by
    // `server-initialize`. Exact per-version ordered scenario sets,
    // `--spec-version` values, and report suffixes are asserted by
    // `compat_runner_scenario_contract_is_exact_per_version` (parser-based);
    // this test guards the two rules that live outside that contract.
    let script = repo_file("scripts/test-mcp-compat.sh");
    assert!(
        script.contains("server-initialize"),
        "test-mcp-compat.sh must run the server-initialize legacy session scenario"
    );
    // Scoped to actual scenario-loop lines: the runner comment explains the
    // missing scenario by name, which must not trip this guard.
    let scenario_loop_references_it = script
        .lines()
        .any(|l| l.contains("for sc in") && l.contains("server-session-lifecycle"));
    assert!(
        !scenario_loop_references_it,
        "test-mcp-compat.sh must not run a server-session-lifecycle scenario \
         (absent from the pinned runner; server-initialize covers the legacy \
         session lifecycle)"
    );
    // Runner exit status must never be suppressed: the runner's global
    // `set -euo pipefail` fails the whole script on any nonzero exit
    // (anchored here on the `set -e` substring).
    assert!(
        script.contains("set -e"),
        "test-mcp-compat.sh must fail on any nonzero runner exit"
    );
}

#[test]
fn inspector_smoke_script_pins_inspector_and_covers_expected_surface() {
    let script = repo_file("scripts/inspector-smoke.mjs");
    assert!(
        script.contains(PINNED_INSPECTOR_PACKAGE),
        "inspector-smoke.mjs must document the pinned {PINNED_INSPECTOR_PACKAGE}"
    );
    for needle in [
        "serverUrl",
        "MCP_AUTO_OPEN_ENABLED",
        "--format",
        "protocolEra",
        "compute_checksum",
        "serial://ports",
        "diagnose_port",
        "interactive_terminal",
    ] {
        assert!(
            script.contains(needle),
            "inspector-smoke.mjs must contain {needle:?}"
        );
    }
    // The default Inspector command must be the lockfile-pinned LOCAL binary
    // (compat/mcp-validation/node_modules/.bin/mcp-inspector) — never npx,
    // never a dynamic package resolution.
    assert!(
        script.contains("LOCKED_INSPECTOR_BIN")
            && script.contains("mcp-validation")
            && script.contains("node_modules"),
        "inspector-smoke.mjs must default to the local locked Inspector \
         binary from compat/mcp-validation"
    );
    // Same rule scoped to non-comment lines: comments may explain the rule
    // ("never npx"), only actual usage counts.
    let npx_usage: Vec<&str> = script
        .lines()
        .filter(|l| {
            l.contains("npx")
                && !l.trim_start().starts_with("//")
                && !l.trim_start().starts_with("/*")
        })
        .collect();
    assert!(
        npx_usage.is_empty(),
        "inspector-smoke.mjs must never use npx in code: {npx_usage:?}"
    );
    // The standalone `--inspector-cmd` option must consume every following
    // argv token (at least one required) — regression guard for the parsing
    // fix; a rewrite that only matches `--inspector-cmd=<path>` or the env
    // var silently breaks the documented multi-token form.
    assert!(
        script.contains("argv.indexOf(\"--inspector-cmd\")"),
        "inspector-smoke.mjs must parse the standalone --inspector-cmd option"
    );
    // The non-interactive description must not claim --stored-auth-only
    // (it is not used; the actual protections are MCP_AUTO_OPEN_ENABLED,
    // non-TTY stdio, and the bounded connect timeout).
    assert!(
        !script.contains("--stored-auth-only"),
        "inspector-smoke.mjs must not claim --stored-auth-only"
    );
    // The smoke must be a hard gate: any assertion failure or nonzero CLI
    // exit leaves the script failing.
    assert!(
        script.contains("process.exitCode = 1"),
        "inspector-smoke.mjs must exit nonzero on assertion failure"
    );
}

#[test]
fn readme_states_dual_protocol_compliance() {
    // The user-facing compliance claim must name both supported protocol
    // versions (2025-11-25 legacy sessions + 2026-07-28 modern discovery)
    // and the one complete local/CI MCP version gate command.
    let readme = repo_file("README.md");
    assert!(
        readme.contains("2025-11-25") && readme.contains("2026-07-28"),
        "README must state compliance with both supported MCP protocol versions"
    );
    assert!(
        readme.contains("scripts/test-mcp-compat.sh"),
        "README must document the one complete MCP version gate command"
    );
}
