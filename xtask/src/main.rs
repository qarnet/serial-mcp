//! `xtask` — repo-local test/build orchestrator for `serial-mcp`.
//!
//! Centralizes the small set of commands an operator or CI run needs
//! in order to take the repo from a clean checkout to a fully tested
//! state, with no surprises. Each subcommand is intentionally thin:
//! it shells out to existing helpers (`cargo`, `fw-build-native`)
//! and the `tests/common/binaries.rs` /
//! `firmware.rs` test helpers via the `cargo test` build. We do not
//! reimplement cargo, west, or our own build pipeline in here.
//!
//! Subcommands:
//!
//! - `xtask build-test-assets`
//!   Build the `serial-mcp` binary plus the `native_sim` firmware.
//!   Pristine firmware build. Safe to run after a clean checkout or
//!   before the first test run.
//!
//! - `xtask test`
//!   Run the same test set CI runs: unit tests + the four
//!   process-level integration suites (stdio, blob resources,
//!   native_sim validation, native_sim lifecycle). Firmware-required
//!   suites are skipped with a clear message if the firmware build
//!   helpers are not on PATH.
//!
//! - `xtask test-all`
//!   Like `test`, plus the HTTP integration suite. The HTTP suite
//!   spawns a real `serial-mcp --transport=http` child process and
//!   also benefits from a built `serial-mcp` binary.
//!
//! - `xtask print-paths`
//!   Print the on-disk paths the test orchestrator resolves for the
//!   serial-mcp binary and the firmware binary.
//!   Useful for debugging test wiring and for AGENTS.md cross-checks.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

const SERIAL_MCP_BIN: &str = "serial-mcp";
const PLAIN_VARIANT: &str = "native_sim";

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

fn print_help() {
    eprintln!(
        "xtask — serial-mcp test/build orchestrator

USAGE:
    xtask <SUBCOMMAND>

SUBCOMMANDS:
    build-test-assets   Build serial-mcp + native_sim firmware
    test                Run unit + process-level integration tests
    test-all            Like 'test', plus the spawned-binary HTTP suite
    print-paths         Print the resolved test-asset paths
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
    run(
        Command::new("fw-build-native").current_dir(&root),
        "fw-build-native",
    )?;
    eprintln!("xtask: build-test-assets complete");
    Ok(())
}

fn test(rest: &[String], include_http: bool) -> Result<()> {
    let root = workspace_root();
    let mut args: Vec<String> = rest.to_vec();
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
    run(&mut unit, "cargo test --lib")?;

    // Process-level integration suites. Each `cargo test --test <foo>`
    // builds the helper into a separate test binary. The
    // native_sim firmware suites have their tests marked
    // `#[ignore = "requires native_sim firmware binary"]` and need
    // `--ignored`; the others run their default tests directly.
    let hardware_suites: &[(&str, bool)] = &[
        ("stdio_integration", false),
        ("blob_resources", false),
        ("native_sim_validation", true),
        ("native_sim_connection_lifecycle", true),
    ];
    for (suite, with_ignored) in hardware_suites {
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .args(["test", "--test", suite, "--locked", "--"]);
        if *with_ignored {
            c.arg("--ignored");
        }
        c.args(&args);
        run(&mut c, &format!("cargo test --test {suite}"))?;
    }

    if include_http {
        // HTTP suite does not need `--ignored` and uses the spawned
        // binary which is the default.
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .args(["test", "--test", "http_integration", "--locked", "--"])
            .args(&args);
        run(&mut c, "cargo test --test http_integration")?;
    }

    if include_http {
        // HTTP suite does not need `--ignored` and uses the spawned
        // binary which is the default.
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .args(["test", "--test", "http_integration", "--locked", "--"])
            .args(&args);
        run(&mut c, "cargo test --test http_integration")?;
    }

    eprintln!("xtask: test complete");
    Ok(())
}

fn print_paths() -> Result<()> {
    let root = workspace_root();
    let bin = root.join("target").join("debug").join(SERIAL_MCP_BIN);
    let plain = root
        .join("build")
        .join(PLAIN_VARIANT)
        .join("firmware")
        .join("zephyr")
        .join("zephyr.exe");
    println!("serial-mcp binary: {}", bin.display());
    println!("firmware:          {}", plain.display());
    println!("\nThese paths mirror tests/common/binaries.rs and tests/common/firmware.rs.");
    Ok(())
}
