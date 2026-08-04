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
    let (action, revision) = ref_part.rsplit_once('@')?;
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
    // The comment identifies the semantic version (`v7`) or the generic
    // rust-toolchain action (`master`); anything else is not a version label.
    let comment = comment.unwrap();
    let comment_ok = comment == "master"
        || comment
            .strip_prefix('v')
            .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()));
    if !comment_ok {
        return Some(format!(
            "external action {action} comment {comment:?} is not a readable \
             version label (expected `# v<N>` or `# master`): {raw:?}"
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
                panic!("{name}:{line_no} {violation}");
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
        external_action_pin_violation("uses: ./.github/workflows/release.yml"),
        None,
        "local reusable workflow references stay exempt"
    );
    for mutable in [
        "uses: actions/checkout@v7",
        "uses: actions/checkout@main",
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", // missing comment
        "uses: actions/checkout@3D3C42E5AAC5BA805825DA76410C181273BA90B1 # v7", // uppercase
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # tag", // non-semver comment
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
}

#[test]
fn compat_runner_pins_packages_and_never_runs_suite_all() {
    // The shared runner is the executable compatibility gate: it must pin the
    // exact conformance package, wire the Inspector smoke script, apply the
    // exact expected-failure baseline path, exercise the historical fixture
    // over BOTH transports, run under `set -euo pipefail`, and never run
    // `--suite all` (comments may explain why it is forbidden).
    let script = repo_file("scripts/test-mcp-compat.sh");
    assert!(
        script.contains(PINNED_CONFORMANCE_PACKAGE),
        "test-mcp-compat.sh must invoke the pinned {PINNED_CONFORMANCE_PACKAGE}"
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
}

#[test]
fn compat_runner_scenario_contract_is_exact_per_version() {
    // The exact version-indexed scenario contract: ordered quoted
    // assignments, exact --spec-version values, and exact report suffixes.
    runner_scenario_contract(&repo_file("scripts/test-mcp-compat.sh"))
        .unwrap_or_else(|e| panic!("runner scenario contract violated: {e}"));
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
        "inspector-smoke.mjs must pin {PINNED_INSPECTOR_PACKAGE} as its npx fallback"
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
