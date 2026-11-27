//! Process-level contract of the CLI (SPEC §4.3/§4.4): validation
//! errors and a bad explicit --config are clap-style errors with exit
//! code 2, and --help lists the whole v1 surface. The daemon itself
//! is never started here: every invocation below fails or prints
//! help before touching PipeWire.

use std::{fs, path::PathBuf, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_gamescope-led-sync");

/// Run the binary with an empty XDG_CONFIG_HOME so config discovery
/// falls through to the built-in defaults, deterministically.
fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env("XDG_CONFIG_HOME", "/nonexistent-gls-cli-test")
        .output()
        .unwrap()
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn missing_explicit_config_is_fatal_with_exit_2() {
    let out = run(&["--config", "/nonexistent-gls-cli-test/config.toml"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("error"), "stderr: {stderr}");
}

#[test]
fn unparseable_explicit_config_is_fatal_with_exit_2() {
    let dir = std::env::temp_dir().join(format!("gls-cli-bad-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join("bad.toml");
    fs::write(&path, "[wled\nport = ").unwrap();
    let out = run(&["--config", path.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr_of(&out));
}

#[test]
fn invalid_flag_value_exits_2_and_names_the_field() {
    let out = run(&["--port", "0"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("wled.port"), "stderr: {stderr}");
}

#[test]
fn unknown_enum_value_exits_2() {
    for args in [["--direction", "sideways"], ["--start", "xx"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2), "args: {args:?}");
    }
}

#[test]
fn counterclockwise_flag_is_removed() {
    let out = run(&["--counterclockwise"]);
    assert_eq!(out.status.code(), Some(2), "the old flag must be rejected");
}

#[test]
fn gap_above_50_exits_2_and_names_the_bound() {
    // The seam hole is capped at 50 LED pitches (the Decky slider's max);
    // the packet length stays count whatever the gap.
    let out = run(&["--gap", "51"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("gap") && stderr.contains("50"),
        "stderr: {stderr}"
    );
    // 50 itself is accepted — covered by the cli::tests unit tests, since a
    // valid run would start the daemon and block this test.
}

#[test]
fn help_lists_the_full_v1_cli_surface() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--config",
        "--setup",
        "--dry-run",
        "--wled",
        "--port",
        "--timeout",
        "--top",
        "--right",
        "--bottom",
        "--left",
        "--start",
        "--direction",
        "--offset",
        "--gap",
        "--fps",
        "--sample-height",
        "--band",
        "--saturation",
        "--gamma",
        "--smooth",
        "--brightness",
        "--once",
        "--dump",
        "--preview",
        "--calibrate",
    ] {
        assert!(stdout.contains(flag), "help must list {flag}:\n{stdout}");
    }
    assert!(
        !stdout.contains("--counterclockwise"),
        "help must not list it"
    );
}

/// --setup with EOF on stdin (non-interactive) must quit cleanly with
/// exit code 0 and write nothing.
#[test]
fn setup_with_eof_on_stdin_quits_without_writing() {
    let dir = std::env::temp_dir().join(format!("gls-setup-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    let out = Command::new(BIN)
        .args(["--setup", "--config", config.to_str().unwrap()])
        .env("XDG_CONFIG_HOME", "/nonexistent-gls-cli-test")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("no config file written"),
        "stdout: {stdout}"
    );
    assert!(!config.exists(), "setup must not write on EOF");
    let _ = fs::remove_dir_all(&dir);
}

/// `--calibrate` without a WLED host must fail cleanly (exit 1, named
/// error) before touching any display or PipeWire: the host check comes
/// first in calibrate::run, so this is safe to run headless.
#[test]
fn calibrate_without_wled_host_fails_cleanly() {
    let out = run(&["--calibrate"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("--calibrate") && stderr.contains("WLED"),
        "stderr: {stderr}"
    );
}

/// `--version` prints `<name> <crate version>` on stdout with exit code 0.
/// The Decky plugin parses exactly this line to compare the installed
/// binary's version with its own (SPEC §11, D14), so the format is a
/// contract: one line, `gamescope-led-sync X.Y.Z`.
#[test]
fn version_flag_prints_the_crate_version() {
    let out = run(&["--version"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}");
    let line = stdout.trim();
    assert!(
        line == format!("gamescope-led-sync {}", env!("CARGO_PKG_VERSION")),
        "expected `gamescope-led-sync {}`, got: {line}",
        env!("CARGO_PKG_VERSION")
    );
}
