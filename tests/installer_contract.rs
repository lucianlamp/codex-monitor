#![cfg(not(windows))]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn foreground_helper_suppresses_empty_polls_and_returns_first_message() {
    let temp = tempfile::tempdir().unwrap();
    let scripts = temp.path().join("agmsg");
    fs::create_dir_all(&scripts).unwrap();
    let inbox = scripts.join("inbox.sh");
    fs::write(
        &inbox,
        r#"#!/usr/bin/env bash
count_file="$AGMSG_TEST_COUNT"
count=0
[[ -f "$count_file" ]] && count=$(cat "$count_file")
count=$((count + 1))
printf '%s\n' "$count" > "$count_file"
if (( count < 3 )); then
  printf 'No new messages.\n'
else
  printf '1 new message(s):\n\n  [now] sender: foreground-ready\n'
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&inbox, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new("bash")
        .arg(repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh"))
        .args(["cdxm", "codex"])
        .env("AGMSG_SCRIPTS_DIR", &scripts)
        .env("AGMSG_TEST_COUNT", temp.path().join("count"))
        .env("CDXM_FOREGROUND_POLL_SECONDS", "0")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foreground-ready"));
    assert!(!stdout.contains("No new messages."));
    assert_eq!(
        fs::read_to_string(temp.path().join("count"))
            .unwrap()
            .trim(),
        "3"
    );
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

    let zshrc = fs::read_to_string(home.path().join(".zshrc")).unwrap();
    assert!(zshrc.contains(&format!(
        "export PATH=\"{}:$PATH\"",
        home.path().join(".codex-monitor/bin").display()
    )));
    assert!(zshrc.contains(&format!(
        "export PATH=\"{}:$PATH\"",
        home.path().join(".agents/bin").display()
    )));
}

#[test]
fn cdxm_launcher_forwards_arguments_and_exit_status() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source")
        .arg(repo_root())
        .arg("--yes")
        .arg("--no-shim")
        .arg("--skip-build")
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let bin = home.path().join(".codex-monitor/bin");
    let primary = bin.join("codex-monitor");
    fs::write(
        &primary,
        "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\"\nexit 23\n",
    )
    .unwrap();
    fs::set_permissions(&primary, fs::Permissions::from_mode(0o755)).unwrap();

    let alias = Command::new(bin.join("cdxm"))
        .args(["one", "two words"])
        .output()
        .unwrap();
    assert_eq!(alias.status.code(), Some(23));
    assert_eq!(
        String::from_utf8(alias.stdout).unwrap().trim(),
        "one two words"
    );
}

#[test]
fn installer_backs_up_and_replaces_foreign_codex_entrypoint() {
    let home = tempfile::tempdir().unwrap();
    let agents_bin = home.path().join(".agents/bin");
    fs::create_dir_all(&agents_bin).unwrap();
    let shim = agents_bin.join("codex");
    let original = "#!/usr/bin/env bash\nprintf 'existing\\n'\n";
    fs::write(&shim, original).unwrap();

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

    // An explicit --install-shim takes over the entrypoint with the codex-monitor
    // shim, but keeps the previous entrypoint as a timestamped backup.
    let shim_text = fs::read_to_string(&shim).unwrap();
    assert!(shim_text.contains("CODEX_MONITOR_SHIM_WRAPPER=1"));

    let backup = fs::read_dir(&agents_bin)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("codex.bak-"))
        .expect("a codex.bak-* backup should exist");
    assert_eq!(fs::read_to_string(backup.path()).unwrap(), original);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Backed up existing"));
}

#[test]
fn installer_rewrites_managed_path_block_with_agents_bin() {
    let home = tempfile::tempdir().unwrap();
    let zshrc = home.path().join(".zshrc");
    fs::write(
        &zshrc,
        format!(
            "before\n# >>> codex-monitor PATH >>>\nexport PATH=\"{}:$PATH\"\n# <<< codex-monitor PATH <<<\nafter\n",
            home.path().join(".codex-monitor/bin").display()
        ),
    )
    .unwrap();

    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source")
        .arg(repo_root())
        .arg("--yes")
        .arg("--no-shim")
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

    let zshrc_text = fs::read_to_string(&zshrc).unwrap();
    assert!(zshrc_text.contains("before"));
    assert!(zshrc_text.contains("after"));
    assert_eq!(
        zshrc_text.matches("# >>> codex-monitor PATH >>>").count(),
        1
    );
    assert!(zshrc_text.contains(&format!(
        "export PATH=\"{}:$PATH\"",
        home.path().join(".codex-monitor/bin").display()
    )));
    assert!(zshrc_text.contains(&format!(
        "export PATH=\"{}:$PATH\"",
        home.path().join(".agents/bin").display()
    )));
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

    for helper in ["whoami.sh", "identities.sh", "delivery.sh"] {
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

fn install_sh() -> String {
    fs::read_to_string(repo_root().join("install.sh")).unwrap()
}

#[test]
fn installer_has_prebuilt_download_path() {
    let s = install_sh();
    assert!(s.contains("releases/latest/download"));
    assert!(s.contains("CDXM_INSTALL_RELEASE_BASE"));
    // verifies a checksum before installing
    assert!(s.contains("shasum") || s.contains("sha256sum"));
    // maps macOS arches to the two darwin targets
    assert!(s.contains("aarch64-apple-darwin"));
    assert!(s.contains("x86_64-apple-darwin"));
    assert!(s.contains("codex-monitor-$target.tar.gz"));
    assert!(s.contains("Prebuilt archive did not contain the expected binaries"));
    // explicit source-build opt-in still exists
    assert!(s.contains("--build-from-source"));
}

#[test]
fn installer_skip_build_still_installs_nothing() {
    // Re-uses the existing skill+shim contract: --skip-build must not download or build.
    let home = tempfile::tempdir().unwrap();
    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--source")
        .arg(repo_root())
        .arg("--yes")
        .arg("--no-shim")
        .arg("--skip-build")
        .env("HOME", home.path())
        .env(
            "CDXM_INSTALL_RELEASE_BASE",
            "http://127.0.0.1:0/should-not-be-used",
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.path().join(".codex-monitor/bin/cdxm").exists());
}
