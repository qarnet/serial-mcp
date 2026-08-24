//! `xtask` provides repo-local build and test commands for `serial-mcp`.
//!
//! Each subcommand delegates to existing `cargo`, `fw-build-native`, and test
//! helpers. It does not reimplement Cargo, west, or the project build pipeline.
//!
//! Subcommands:
//!
//! - `xtask build-test-assets`
//!   Build the `serial-mcp` binary and the `native_sim` firmware. The firmware
//!   build is pristine and can run after a clean checkout or before tests.
//!
//! - `xtask test`
//!   Run unit tests and four process-level integration suites: stdio, blob
//!   resources, native_sim validation, and native_sim lifecycle. Shared test
//!   helpers build missing assets.
//!
//! - `xtask test-all`
//!   Run the same suites as `test`, plus the HTTP integration suite. The HTTP
//!   suite spawns a real `serial-mcp --transport=http` child process and uses
//!   the built `serial-mcp` binary.
//!
//! - `xtask print-paths`
//!   Print the on-disk paths resolved for the `serial-mcp` binary and firmware
//!   binary. Use this to debug test wiring and compare paths with AGENTS.md.
//!
//! - `xtask agent-eval [--output-dir PATH] [--baseline PATH] [--write-baseline PATH]`
//!   Run the deterministic agent-interface evaluation. It measures catalog
//!   bytes from the live `tools/list` catalog, evaluates fixed call-shape
//!   scenarios, and applies fixed decision thresholds. It writes `report.json`
//!   and `report.md` under `target/agent-interface-eval/` by default. It uses
//!   no network, user config, or timestamps.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

mod agent_eval;

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

/// Parse `agent-eval` flags: `--output-dir <path>`, `--baseline <path>`, and
/// `--write-baseline <path>`. Each flag also accepts `--flag=<path>`.
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
    build-test-assets   Build serial-mcp + native_sim firmware
    test                Run unit + process-level integration tests
    test-all            Like 'test', plus the spawned-binary HTTP suite
    print-paths         Print the resolved test-asset paths
    agent-eval          Run the deterministic agent-interface evaluation
                        (--output-dir PATH, --baseline PATH,
                        --write-baseline PATH)
    help                Print this message
"
    );
}

/// Return workspace root from xtask's compile-time manifest directory.
///
/// `CARGO_MANIFEST_DIR` is xtask's compile-time manifest directory. Its parent
/// is the workspace root, so resolution does not depend on process cwd.
fn workspace_root() -> PathBuf {
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
    // `cargo test` separates program arguments from test-runner arguments with
    // a literal `--`. Always pass `--test-threads=1` or the caller's value on
    // the runner side.
    let has_threads = args
        .iter()
        .any(|a| a == "--test-threads" || a.starts_with("--test-threads="));
    if !has_threads {
        args.push("--test-threads=1".to_string());
    }

    // Run library unit tests.
    let mut unit = Command::new("cargo");
    unit.current_dir(&root)
        .args(["test", "--lib", "--locked", "--"])
        .args(&args);
    run(&mut unit, "cargo test --lib")?;

    // Run process-level integration suites. Each `cargo test --test <foo>`
    // builds the helper into a separate test binary. The native_sim firmware
    // suites mark their tests with `#[ignore = "requires native_sim firmware binary"]`
    // and need `--ignored`; other suites run their default tests directly.
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
        // Run the HTTP suite without `--ignored`. It uses the spawned binary.
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
