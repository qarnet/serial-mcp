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
    // Phase 5: the README must teach capture_boot as the boot/reset path,
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
    // Phase 4: the normal workflow is discover → bare open → transact →
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
    // Phase 4 decision tree, not a flat tool list.
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
    // Phase 6: README must teach the disabled-by-default capture store,
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
