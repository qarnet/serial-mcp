//! Guards against tool-count drift between the code and its descriptions.
//!
//! The number of MCP tools is hardcoded in three prose surfaces (README.md,
//! Cargo.toml `description`, server.json `description`) and enumerated in the
//! README capabilities line. The registry listing shipped "12 tools" while the
//! server had 22 because nothing tied these together — this test does.
//!
//! Source of truth: the number of `#[tool(` attributes in `src/server.rs`.

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
}

#[test]
fn readme_tool_count_matches_code() {
    let n = tool_count();
    let readme = repo_file("README.md");
    let marker = format!("**{n} tools:**");
    assert!(
        readme.contains(&marker),
        "README.md must contain {marker:?} — the server defines {n} tools"
    );
}

#[test]
fn readme_tool_list_matches_count() {
    let n = tool_count();
    let readme = repo_file("README.md");
    let line = readme
        .lines()
        .find(|l| l.contains("tools:**"))
        .expect("README.md must have a '**N tools:**' capabilities line");
    let list = line
        .split("tools:**")
        .nth(1)
        .expect("capabilities line must have content after 'tools:**'");
    let listed = list.split(',').count();
    assert_eq!(
        listed, n,
        "README capabilities line enumerates {listed} tools but the server \
         defines {n}"
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
fn readme_readfrom_examples_use_tagged_wire_form() {
    // The `ReadFrom` wire format is a tagged object (`{"type":"now"}`), not a
    // bare string. README prose that teaches the `from` parameter must not
    // regress to string shorthand — agents copy these examples verbatim.
    let readme = repo_file("README.md");
    let line = readme
        .lines()
        .find(|l| l.contains("`from` parameter"))
        .expect("README.md must describe the `from` parameter");
    for tagged in [
        r#"{"type":"cursor"}"#,
        r#"{"type":"now"}"#,
        r#"{"type":"buffer_start"}"#,
        r#"{"type":"offset","offset":N}"#,
    ] {
        assert!(
            line.contains(tagged),
            "README `from` line must contain {tagged}: {line}"
        );
    }
    for shorthand in [
        "from: \"now\"",
        "from: \"cursor\"",
        "from: \"buffer_start\"",
    ] {
        assert!(
            !line.contains(shorthand),
            "README `from` line must not advertise {shorthand}: {line}"
        );
    }
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
fn agent_config_readme_anchor_is_valid() {
    // docs/agent-config.md referenced the removed `#how-rx-works` anchor.
    let config = repo_file("docs/agent-config.md");
    assert!(
        !config.contains("#how-rx-works"),
        "agent-config.md must not reference the removed README anchor"
    );
    assert!(
        config.contains("../README.md#capabilities"),
        "agent-config.md must link a valid README anchor"
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
fn readme_teaches_capture_export_contract() {
    // README must teach the disabled-by-default capture store,
    // the filename-only export_log contract, and the no-overwrite rule.
    let readme = repo_file("README.md");
    assert!(
        readme.contains("--capture-dir"),
        "README must document --capture-dir"
    );
    assert!(
        readme.contains("--capture-max-file-bytes")
            && readme.contains("--capture-max-total-bytes")
            && readme.contains("--capture-max-files"),
        "README must document all three capture quotas"
    );
    let export_section = readme
        .find("`export_log`")
        .map(|i| &readme[i..i + 3000])
        .unwrap_or_default();
    assert!(
        export_section.contains("filename"),
        "README export_log teaching must say path is a filename: {export_section}"
    );
    assert!(
        export_section.contains("never overwrites") || export_section.contains("no overwrite"),
        "README export_log teaching must state the no-overwrite rule"
    );
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
        build.contains("actions/upload-artifact@v7")
            && build.contains("name: serial-mcp-${{ matrix.artifact_name_suffix }}"),
        "build job must upload deterministic named artifacts per platform"
    );
    let publish = job_section(&release, "publish-release");
    assert!(
        publish.contains("actions/download-artifact@v8"),
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
