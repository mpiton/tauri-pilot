use assert_cmd::Command;

/// `tauri-pilot --version` must print the CLI version. The flag was missing
/// entirely until issue #135 surfaced it (a caller couldn't tell which CLI
/// build they were running).
#[test]
fn test_version_flag_prints_cli_version() {
    let mut cmd = Command::cargo_bin("tauri-pilot").expect("tauri-pilot binary builds");
    let assert = cmd.arg("--version").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert_eq!(
        stdout.trim_end(),
        format!("tauri-pilot {}", env!("CARGO_PKG_VERSION")),
        "expected `--version` to print `tauri-pilot <version>`, got: {stdout}",
    );
}
