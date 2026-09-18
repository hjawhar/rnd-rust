//! Black-box CLI integration tests for the `perception` binary.
//!
//! These tests invoke the compiled binary via `std::process::Command` and
//! verify exit codes and output strings. No internal crate imports are needed.

use std::process::Command;

/// Resolve the path to the `perception` binary built by `cargo test`.
fn perception_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_perception"))
}

#[test]
fn test_help_output() {
    let output = perception_bin()
        .arg("--help")
        .output()
        .expect("failed to execute perception --help");

    assert!(output.status.success(), "perception --help must exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("perception"),
        "help output must mention 'perception', got:\n{stdout}"
    );
    assert!(
        stdout.contains("run"),
        "help output must list the 'run' subcommand, got:\n{stdout}"
    );
    assert!(
        stdout.contains("info"),
        "help output must list the 'info' subcommand, got:\n{stdout}"
    );
    assert!(
        stdout.contains("download"),
        "help output must list the 'download' subcommand, got:\n{stdout}"
    );
}

#[test]
fn test_version_flag() {
    let output = perception_bin()
        .arg("--version")
        .output()
        .expect("failed to execute perception --version");

    assert!(output.status.success(), "perception --version must exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version output must contain the crate version '{}', got:\n{stdout}",
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn test_info_command() {
    let output = perception_bin()
        .arg("info")
        .output()
        .expect("failed to execute perception info");

    assert!(output.status.success(), "perception info must exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Perception"),
        "info output must contain 'Perception', got:\n{stdout}"
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "info output must contain the crate version, got:\n{stdout}"
    );
}

#[test]
fn test_run_missing_config() {
    let output = perception_bin()
        .args(["--config", "/tmp/perception_nonexistent_config_42.toml", "run"])
        .output()
        .expect("failed to execute perception run");

    assert!(
        !output.status.success(),
        "perception run with missing config must fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("perception_nonexistent_config_42.toml")
            || combined.contains("No such file")
            || combined.contains("not found")
            || combined.contains("Error"),
        "error output must reference the missing config or indicate failure, got:\n{combined}"
    );
}

#[test]
fn test_no_subcommand_shows_help() {
    let output = perception_bin()
        .output()
        .expect("failed to execute perception (no args)");

    // clap exits non-zero when a required subcommand is missing.
    assert!(
        !output.status.success(),
        "perception with no subcommand should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage") || stderr.contains("usage") || stderr.contains("USAGE"),
        "stderr should contain usage info when no subcommand is given, got:\n{stderr}"
    );
}
