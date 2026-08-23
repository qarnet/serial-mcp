//! `xtask` — repo-local test/build orchestrator for `serial-mcp`.
//!
//! Centralizes the small set of commands an operator or CI run needs
//! in order to take the repo from a clean checkout to a fully tested
//! state, with no surprises. Each subcommand is intentionally thin:
//! it shells out to existing helpers (`cargo`) and the
//! `tests/common/binaries.rs` test helper via the `cargo test` build. We do
//! not reimplement cargo or our own build pipeline in here.
//!
//! Subcommands:
//!
//! - `xtask build-test-assets`
//!   Build the `serial-mcp` binary. Safe to run after a clean checkout or
//!   before the first test run.
//!
//! - `xtask test`
//!   Run unit tests plus required Rust PTY fixture, command, framing, and
//!   protocol suites, then stdio/blob. Missing test assets are built through
//!   the shared test helper.
//!
//! - `xtask test-all`
//!   Like `test`, plus the HTTP integration suite. The HTTP suite
//!   spawns a real `serial-mcp --transport=http` child process and
//!   also benefits from a built `serial-mcp` binary.
//!
//! - `xtask print-paths`
//!   Print the on-disk path the test orchestrator resolves for the
//!   serial-mcp binary.
//!   Useful for debugging test wiring and for AGENTS.md cross-checks.
//!
//! - `xtask agent-eval [--output-dir PATH] [--baseline PATH] [--write-baseline PATH]`
//!   Run the deterministic agent-interface evaluation: catalog bytes from the
//!   live `tools/list` catalog plus fixed call-shape scenarios and decision
//!   thresholds. Writes `report.json` and `report.md` under
//!   `target/agent-interface-eval/` by default. No network, user config, or
//!   timestamps.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

mod agent_eval;

const SERIAL_MCP_BIN: &str = "serial-mcp";
const KEEP_TEST_TARGET: &str = "--keep-test-target";
const RESERVED_TARGET_ATTEMPTS: u32 = 128;

#[derive(Debug, Clone)]
struct ReservedTestTarget {
    workspace_root: PathBuf,
    target_parent: PathBuf,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildEnvironment {
    cargo_target_dir: PathBuf,
    serial_mcp_bin: PathBuf,
}

impl ChildEnvironment {
    fn for_target(target: &ReservedTestTarget) -> Self {
        Self {
            cargo_target_dir: target.path.clone(),
            serial_mcp_bin: serial_mcp_binary_path(&target.path),
        }
    }

    fn apply(&self, command: &mut Command) {
        command
            .env("CARGO_TARGET_DIR", &self.cargo_target_dir)
            .env("SERIAL_MCP_BIN", &self.serial_mcp_bin);
    }
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("xtask: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let sub = argv.get(1).map(String::as_str).unwrap_or("help");
    let rest: &[String] = argv.get(2..).unwrap_or(&[]);
    match sub {
        "build-test-assets" => build_test_assets(rest),
        "test" => test(rest, false),
        "test-all" => test(rest, true),
        "print-paths" => print_paths(),
        "agent-eval" => agent_eval(rest),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            print_help();
            std::process::exit(2);
        }
    }
}

/// Parse `agent-eval` flags: `--output-dir <path>`, `--baseline <path>`,
/// `--write-baseline <path>` (each optionally `--flag=<path>`).
fn agent_eval(rest: &[String]) -> Result<()> {
    let mut options = agent_eval::EvalOptions::default();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = arg
            .split_once('=')
            .map(|(f, v)| (f, Some(v.to_string())))
            .unwrap_or((arg.as_str(), None));
        let mut value = || -> Result<String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => it
                    .next()
                    .cloned()
                    .with_context(|| format!("{flag} requires a value")),
            }
        };
        match flag {
            "--output-dir" => options.output_dir = Some(PathBuf::from(value()?)),
            "--baseline" => options.baseline = Some(PathBuf::from(value()?)),
            "--write-baseline" => options.write_baseline = Some(PathBuf::from(value()?)),
            other => anyhow::bail!("unknown agent-eval flag: {other}"),
        }
    }
    agent_eval::run(&options)
}

fn print_help() {
    eprintln!(
        "xtask — serial-mcp test/build orchestrator

USAGE:
    xtask <SUBCOMMAND>

SUBCOMMANDS:
    build-test-assets   Build serial-mcp
    test                Run unit + Rust PTY integration tests
                        (optional --keep-test-target)
    test-all            Like 'test', plus the spawned-binary HTTP suite
                        (optional --keep-test-target)
    print-paths         Print the resolved test-asset paths
    agent-eval          Run the deterministic agent-interface evaluation
                        (--output-dir PATH, --baseline PATH,
                        --write-baseline PATH)
    help                Print this message
"
    );
}

fn workspace_root() -> PathBuf {
    // The xtask binary lives at <repo>/xtask/. We resolve the
    // workspace root by walking up from the binary's own source path
    // (compile-time constant), not from the process cwd, so the
    // behavior is independent of where the user invoked the binary.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(PathBuf::from).unwrap_or(manifest)
}

fn canonical_workspace_root() -> Result<PathBuf> {
    let root = workspace_root();
    validate_workspace_root_path(&root)?;
    fs::canonicalize(&root)
        .with_context(|| format!("cannot canonicalize workspace root {}", root.display()))
}

fn validate_workspace_root_path(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("cannot inspect workspace root {}", root.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("workspace root {} is a symlink", root.display());
    }
    if !metadata.is_dir() {
        anyhow::bail!("workspace root {} is not a directory", root.display());
    }
    Ok(())
}

fn validate_target_parent_path(workspace_root: &Path, target_parent: &Path) -> Result<()> {
    let expected = workspace_root.join("target");
    if target_parent != expected {
        anyhow::bail!(
            "target parent {} is not direct child {}",
            target_parent.display(),
            expected.display()
        );
    }
    let metadata = fs::symlink_metadata(target_parent).with_context(|| {
        format!(
            "cannot inspect Cargo target parent {}",
            target_parent.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Cargo target parent {} is a symlink",
            target_parent.display()
        );
    }
    if !metadata.is_dir() {
        anyhow::bail!(
            "Cargo target parent {} is not a directory",
            target_parent.display()
        );
    }
    Ok(())
}

fn ensure_target_parent(workspace_root: &Path) -> Result<PathBuf> {
    let target_parent = workspace_root.join("target");
    match fs::symlink_metadata(&target_parent) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(&target_parent).with_context(|| {
                format!(
                    "cannot create Cargo target parent {}",
                    target_parent.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "cannot inspect Cargo target parent {}",
                    target_parent.display()
                )
            });
        }
    }
    validate_target_parent_path(workspace_root, &target_parent)?;
    Ok(target_parent)
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_reserved_target_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".xtask-test-") else {
        return false;
    };
    let mut fields = rest.split('-');
    let valid = fields.next().is_some_and(is_decimal)
        && fields.next().is_some_and(is_decimal)
        && fields.next().is_some_and(is_decimal);
    valid && fields.next().is_none()
}

fn reserved_target_name(pid: u32, unix_nanos: u128, attempt: u32) -> String {
    format!(".xtask-test-{pid}-{unix_nanos}-{attempt}")
}

fn validate_reserved_candidate(
    workspace_root: &Path,
    target_parent: &Path,
    candidate: &Path,
    require_directory: bool,
) -> Result<()> {
    validate_workspace_root_path(workspace_root)?;
    validate_target_parent_path(workspace_root, target_parent)?;
    if candidate.parent() != Some(target_parent) {
        anyhow::bail!(
            "reserved test target {} is not direct child of {}",
            candidate.display(),
            target_parent.display()
        );
    }
    let name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .with_context(|| {
            format!(
                "reserved test target {} has no valid name",
                candidate.display()
            )
        })?;
    if !valid_reserved_target_name(name) {
        anyhow::bail!(
            "reserved test target {} has malformed name",
            candidate.display()
        );
    }
    if require_directory {
        let metadata = fs::symlink_metadata(candidate).with_context(|| {
            format!(
                "cannot inspect reserved test target {}",
                candidate.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!("reserved test target {} is a symlink", candidate.display());
        }
        if !metadata.is_dir() {
            anyhow::bail!(
                "reserved test target {} is not a directory",
                candidate.display()
            );
        }
    }
    Ok(())
}

fn serial_mcp_binary_path(target: &Path) -> PathBuf {
    target
        .join("debug")
        .join(format!("{SERIAL_MCP_BIN}{}", std::env::consts::EXE_SUFFIX))
}

impl ReservedTestTarget {
    fn reserve(workspace_root: &Path) -> Result<Self> {
        let workspace_root = fs::canonicalize(workspace_root).with_context(|| {
            format!(
                "cannot canonicalize workspace root {}",
                workspace_root.display()
            )
        })?;
        validate_workspace_root_path(&workspace_root)?;
        let target_parent = ensure_target_parent(&workspace_root)?;
        let unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_nanos();
        let pid = std::process::id();

        for attempt in 0..RESERVED_TARGET_ATTEMPTS {
            let name = reserved_target_name(pid, unix_nanos, attempt);
            let candidate = target_parent.join(name);
            validate_reserved_candidate(&workspace_root, &target_parent, &candidate, false)?;
            match fs::create_dir(&candidate) {
                Ok(()) => {
                    let reserved = Self {
                        workspace_root: workspace_root.clone(),
                        target_parent: target_parent.clone(),
                        path: candidate,
                    };
                    reserved.validate_for_cleanup()?;
                    return Ok(reserved);
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(&candidate).with_context(|| {
                        format!("cannot inspect colliding target {}", candidate.display())
                    })?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        anyhow::bail!(
                            "reserved target collision {} is a symlink or non-directory",
                            candidate.display()
                        );
                    }
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "cannot reserve disposable test target {}",
                            candidate.display()
                        )
                    });
                }
            }
        }

        anyhow::bail!(
            "could not reserve disposable test target below {} after {} attempts",
            target_parent.display(),
            RESERVED_TARGET_ATTEMPTS
        )
    }

    fn validate_for_cleanup(&self) -> Result<()> {
        validate_reserved_candidate(&self.workspace_root, &self.target_parent, &self.path, true)
    }

    fn cleanup_after_success(&self) -> Result<()> {
        if let Err(error) = self.validate_for_cleanup() {
            eprintln!(
                "xtask: keeping test target after cleanup failure: {}",
                self.path.display()
            );
            return Err(error);
        }
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "xtask: keeping test target after cleanup failure: {}",
                self.path.display()
            );
            return Err(error).with_context(|| {
                format!(
                    "failed to remove successful test target {}",
                    self.path.display()
                )
            });
        }
        eprintln!(
            "xtask: removed successful test target: {}",
            self.path.display()
        );
        Ok(())
    }
}

fn extract_keep_test_target(rest: &[String]) -> Result<(Vec<String>, bool)> {
    let mut forwarded = Vec::with_capacity(rest.len());
    let mut keep = false;
    for argument in rest {
        if argument == KEEP_TEST_TARGET {
            keep = true;
        } else if argument.starts_with("--keep-test-target=") {
            anyhow::bail!("{KEEP_TEST_TARGET} does not accept a value: {argument}");
        } else {
            forwarded.push(argument.clone());
        }
    }
    Ok((forwarded, keep))
}

fn finish_test_run(target: &ReservedTestTarget, result: Result<()>, keep: bool) -> Result<()> {
    match result {
        Ok(()) if keep => {
            eprintln!(
                "xtask: keeping test target by request: {}",
                target.path.display()
            );
            Ok(())
        }
        Ok(()) => target.cleanup_after_success(),
        Err(error) => {
            eprintln!(
                "xtask: keeping failed test target: {}",
                target.path.display()
            );
            Err(error)
        }
    }
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    eprintln!("xtask: $ {cmd:?}");
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to spawn {what}"))?;
    if !status.success() {
        anyhow::bail!("{what} exited with {status}");
    }
    Ok(())
}

fn build_test_assets(rest: &[String]) -> Result<()> {
    let root = workspace_root();
    let mut cargo = Command::new("cargo");
    cargo
        .current_dir(&root)
        .args(["build", "--bin", SERIAL_MCP_BIN]);
    if let Some(profile) = rest.first() {
        cargo.arg(profile);
    }
    run(&mut cargo, "cargo build --bin serial-mcp")?;
    eprintln!("xtask: build-test-assets complete");
    Ok(())
}

fn test(rest: &[String], include_http: bool) -> Result<()> {
    let (runner_args, keep_test_target) = extract_keep_test_target(rest)?;
    let root = canonical_workspace_root()?;
    let reserved = ReservedTestTarget::reserve(&root)?;
    eprintln!(
        "xtask: reserved disposable test target: {}",
        reserved.path.display()
    );
    finish_test_run(
        &reserved,
        run_test_suites(&root, &runner_args, include_http, &reserved),
        keep_test_target,
    )
}

fn run_test_suites(
    root: &Path,
    runner_args: &[String],
    include_http: bool,
    reserved: &ReservedTestTarget,
) -> Result<()> {
    let mut build = Command::new("cargo");
    build
        .current_dir(root)
        .args(["build", "--bin", SERIAL_MCP_BIN, "--locked"])
        .env("CARGO_TARGET_DIR", &reserved.path);
    run(&mut build, "cargo build --bin serial-mcp --locked")?;

    let environment = ChildEnvironment::for_target(reserved);
    let binary_metadata = fs::symlink_metadata(&environment.serial_mcp_bin).with_context(|| {
        format!(
            "prebuilt serial-mcp binary is missing at {}",
            environment.serial_mcp_bin.display()
        )
    })?;
    if binary_metadata.file_type().is_symlink() || !binary_metadata.is_file() {
        anyhow::bail!(
            "prebuilt serial-mcp binary is not a regular file at {}",
            environment.serial_mcp_bin.display()
        );
    }

    let mut args: Vec<String> = runner_args.to_vec();
    // cargo test separates program args from test-runner args with
    // a literal `--`. We always pass `--test-threads=1` (or the
    // caller's value) on the runner side.
    let has_threads = args
        .iter()
        .any(|a| a == "--test-threads" || a.starts_with("--test-threads="));
    if !has_threads {
        args.push("--test-threads=1".to_string());
    }

    // Library unit tests
    let mut unit = Command::new("cargo");
    unit.current_dir(&root)
        .args(["test", "--lib", "--locked", "--"])
        .args(&args);
    environment.apply(&mut unit);
    run(&mut unit, "cargo test --lib")?;

    // Process-level integration suites. Each `cargo test --test <foo>`
    // builds the helper into a separate test binary. The required real-PTY
    // replacement suites run before the normal stdio/blob suites.
    let integration_suites: &[(&str, bool)] = &[
        ("device_fixture", false),
        ("device_command_parity", false),
        ("device_framing_parity", false),
        ("device_protocol_parity", false),
        ("stdio_integration", false),
        ("blob_resources", false),
    ];
    for (suite, with_ignored) in integration_suites {
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .args(["test", "--test", suite, "--locked", "--"]);
        if *with_ignored {
            c.arg("--ignored");
        }
        c.args(&args);
        environment.apply(&mut c);
        run(&mut c, &format!("cargo test --test {suite}"))?;
    }

    if include_http {
        // HTTP suite does not need `--ignored` and uses the spawned
        // binary which is the default.
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .args(["test", "--test", "http_integration", "--locked", "--"])
            .args(&args);
        environment.apply(&mut c);
        run(&mut c, "cargo test --test http_integration")?;
    }

    eprintln!("xtask: test complete");
    Ok(())
}

fn print_paths() -> Result<()> {
    let root = workspace_root();
    let bin = root.join("target").join("debug").join(SERIAL_MCP_BIN);
    println!("serial-mcp binary: {}", bin.display());
    println!("\nThis path mirrors tests/common/binaries.rs.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_workspace(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "serial-mcp-xtask-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn create_target_parent(root: &Path) -> PathBuf {
        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        target
    }

    #[test]
    fn keep_flag_extraction_is_exact_and_repeated_flags_are_harmless() {
        let input = vec![
            "--keep-test-target".to_string(),
            "--test-threads=1".to_string(),
            "--keep-test-target".to_string(),
            "suite-filter".to_string(),
        ];
        let (forwarded, keep) = extract_keep_test_target(&input).unwrap();
        assert!(keep);
        assert_eq!(
            forwarded,
            vec!["--test-threads=1".to_string(), "suite-filter".to_string()]
        );
        let error = extract_keep_test_target(&["--keep-test-target=value".to_string()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not accept a value"));
    }

    #[test]
    fn reserved_name_and_direct_child_validation_are_strict() {
        assert!(valid_reserved_target_name(".xtask-test-1-2-3"));
        assert!(!valid_reserved_target_name(".xtask-test-1-2"));
        assert!(!valid_reserved_target_name(".xtask-test-1-two-3"));
        assert!(!valid_reserved_target_name(".xtask-test-1-2-3-4"));
        assert!(!valid_reserved_target_name("xtask-test-1-2-3"));

        let root = temporary_workspace("name");
        let target = create_target_parent(&root);
        let candidate = target.join(".xtask-test-1-2-3");
        validate_reserved_candidate(&root, &target, &candidate, false).unwrap();

        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let escaped = outside.join(".xtask-test-1-2-3");
        assert!(validate_reserved_candidate(&root, &target, &escaped, false).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn target_parent_and_candidate_non_directories_are_rejected() {
        let root = temporary_workspace("nondirectory");
        fs::write(root.join("target"), b"not a directory").unwrap();
        assert!(ReservedTestTarget::reserve(&root).is_err());
        fs::remove_dir_all(&root).unwrap();

        let root = temporary_workspace("candidate");
        let target = create_target_parent(&root);
        let candidate = target.join(".xtask-test-1-2-3");
        fs::write(&candidate, b"not a directory").unwrap();
        assert!(validate_reserved_candidate(&root, &target, &candidate, true).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_parent_and_candidate_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = temporary_workspace("target-symlink");
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("target")).unwrap();
        assert!(ReservedTestTarget::reserve(&root).is_err());
        fs::remove_dir_all(&root).unwrap();

        let root = temporary_workspace("candidate-symlink");
        let target = create_target_parent(&root);
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let candidate = target.join(".xtask-test-1-2-3");
        symlink(&outside, &candidate).unwrap();
        assert!(validate_reserved_candidate(&root, &target, &candidate, true).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reservation_creates_unique_direct_absolute_children() {
        let root = temporary_workspace("reserve");
        let first = ReservedTestTarget::reserve(&root).unwrap();
        let second = ReservedTestTarget::reserve(&root).unwrap();
        assert_ne!(first.path, second.path);
        assert!(first.path.is_absolute());
        assert_eq!(first.path.parent(), Some(first.target_parent.as_path()));
        assert_eq!(second.path.parent(), Some(second.target_parent.as_path()));
        assert!(valid_reserved_target_name(
            first.path.file_name().unwrap().to_str().unwrap()
        ));
        assert!(fs::symlink_metadata(&first.path).unwrap().is_dir());
        assert!(fs::symlink_metadata(&second.path).unwrap().is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_environment_has_exact_absolute_paths_and_suffix() {
        let root = temporary_workspace("environment");
        let reserved = ReservedTestTarget::reserve(&root).unwrap();
        let environment = ChildEnvironment::for_target(&reserved);
        assert_eq!(environment.cargo_target_dir, reserved.path);
        assert_eq!(
            environment.serial_mcp_bin,
            reserved
                .path
                .join("debug")
                .join(format!("serial-mcp{}", std::env::consts::EXE_SUFFIX))
        );
        assert!(environment.cargo_target_dir.is_absolute());
        assert!(environment.serial_mcp_bin.is_absolute());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_finalization_removes_only_reserved_child() {
        let root = temporary_workspace("success");
        let reserved = ReservedTestTarget::reserve(&root).unwrap();
        let sibling = reserved.target_parent.join("sibling-marker");
        fs::write(&sibling, b"keep").unwrap();
        fs::write(reserved.path.join("run-marker"), b"remove").unwrap();
        reserved.cleanup_after_success().unwrap();
        assert!(!reserved.path.exists());
        assert!(sibling.is_file());
        assert!(reserved.target_parent.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_and_keep_requested_runs_retain_reserved_target() {
        let root = temporary_workspace("failed");
        let reserved = ReservedTestTarget::reserve(&root).unwrap();
        let error =
            finish_test_run(&reserved, Err(anyhow::anyhow!("child failed")), false).unwrap_err();
        assert_eq!(error.to_string(), "child failed");
        assert!(reserved.path.is_dir());
        fs::remove_dir_all(&root).unwrap();

        let root = temporary_workspace("keep");
        let reserved = ReservedTestTarget::reserve(&root).unwrap();
        finish_test_run(&reserved, Ok(()), true).unwrap();
        assert!(reserved.path.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_rejects_malformed_and_outside_candidates_before_deletion() {
        let root = temporary_workspace("cleanup-path");
        let target = create_target_parent(&root);
        let marker = target.join("sibling-marker");
        fs::write(&marker, b"keep").unwrap();
        let malformed = ReservedTestTarget {
            workspace_root: root.clone(),
            target_parent: target.clone(),
            path: target.join("not-a-reserved-target"),
        };
        assert!(malformed.cleanup_after_success().is_err());
        assert!(marker.is_file());

        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let escaped = ReservedTestTarget {
            workspace_root: root.clone(),
            target_parent: target.clone(),
            path: outside.join(".xtask-test-1-2-3"),
        };
        assert!(escaped.cleanup_after_success().is_err());
        assert!(marker.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_symlink_candidate_before_deletion() {
        use std::os::unix::fs::symlink;

        let root = temporary_workspace("cleanup-symlink");
        let target = create_target_parent(&root);
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let marker = outside.join("marker");
        fs::write(&marker, b"keep").unwrap();
        let candidate = target.join(".xtask-test-1-2-3");
        symlink(&outside, &candidate).unwrap();
        let reserved = ReservedTestTarget {
            workspace_root: root.clone(),
            target_parent: target,
            path: candidate,
        };
        assert!(reserved.cleanup_after_success().is_err());
        assert!(marker.is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
