#![cfg(not(windows))]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn installer_installs_skill_and_optional_shim_without_building() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source")
        .arg(repo_root())
        .arg("--yes")
        .arg("--install-shim")
        .arg("--skip-build")
        .env("HOME", home.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let skill = home.path().join(".codex/skills/codex-monitor/SKILL.md");
    assert!(skill.exists());
    assert!(fs::read_to_string(&skill)
        .unwrap()
        .contains("name: codex-monitor"));

    let shim = home.path().join(".agents/bin/codex");
    let shim_text = fs::read_to_string(&shim).unwrap();
    assert!(shim_text.contains("CODEX_MONITOR_SHIM_WRAPPER=1"));
    assert!(shim_text.contains("Codex monitor shim"));
    assert!(shim_text.contains("app-server --listen"));
    assert!(fs::metadata(&shim).unwrap().permissions().mode() & 0o111 != 0);
}

#[test]
fn installer_never_overwrites_existing_codex_entrypoint() {
    let home = tempfile::tempdir().unwrap();
    let agents_bin = home.path().join(".agents/bin");
    fs::create_dir_all(&agents_bin).unwrap();
    let shim = agents_bin.join("codex");
    fs::write(&shim, "#!/usr/bin/env bash\nprintf 'existing\\n'\n").unwrap();
    let before = fs::read_to_string(&shim).unwrap();

    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source")
        .arg(repo_root())
        .arg("--yes")
        .arg("--install-shim")
        .arg("--skip-build")
        .env("HOME", home.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&shim).unwrap(), before);
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("leaving existing codex entrypoint untouched"));
}
