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

#[test]
fn agmsg_apply_does_not_treat_stale_thread_consumer_as_active() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let fake_bin = temp.path().join("bin");
    let fake_agmsg = temp.path().join("agmsg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&fake_agmsg).unwrap();

    let fake_cdxm = fake_bin.join("cdxm");
    fs::write(
        &fake_cdxm,
        r#"#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  *"agmsg doctor"*)
    printf 'doctor\tconsumer\ttarget\tpid=52232\tkind=codex-monitor-agmsg-watch\tteam=emeria\tname=codex\tthread=old-thread\tcommand=cdxm agmsg watch --team emeria --name codex --thread old-thread\n'
    ;;
  *"monitor watch agmsg"*"--dry-run"*)
    printf 'dry-run ok\n'
    ;;
  *)
    printf 'unexpected cdxm args: %s\n' "$*" >&2
    exit 64
    ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_cdxm, fs::Permissions::from_mode(0o755)).unwrap();

    for helper in ["whoami.sh", "identities.sh"] {
        let path = fake_agmsg.join(helper);
        fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let apply_script = temp.path().join("cdxm-agmsg-apply.sh");
    fs::copy(
        repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-apply.sh"),
        &apply_script,
    )
    .unwrap();
    fs::set_permissions(&apply_script, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new("bash")
        .arg(&apply_script)
        .arg(&project)
        .args(["--team", "emeria", "--name", "codex", "--dry-run-only"])
        .env("HOME", &home)
        .env("CODEX_THREAD_ID", "current-thread")
        .env("CODEX_MONITOR_BIN", &fake_cdxm)
        .env("AGMSG_SCRIPTS_DIR", &fake_agmsg)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("stale thread pin"));
    assert!(stdout.contains("old-thread"));
    assert!(stdout.contains("current-thread"));
    assert!(stdout.contains("dry-run ok"));
    assert!(!stdout.contains("already has an active target consumer"));
}
