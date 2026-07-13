use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn bash_path() -> PathBuf {
    #[cfg(windows)]
    {
        for candidate in [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files\Git\usr\bin\bash.exe",
        ] {
            if Path::new(candidate).is_file() {
                return PathBuf::from(candidate);
            }
        }
        panic!("Git Bash is required for App hook integration tests");
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/bin/bash")
    }
}

fn stop_hook(
    binary: &str,
    root: &Path,
    helper: &Path,
    bash: &Path,
    payload: &Value,
) -> std::process::Output {
    let mut command = Command::new(binary);
    command
        .arg("__app-stop-hook")
        .env("CDXM_APP_HOOK_ROOT", root)
        .env("CDXM_APP_HOOK_FOREGROUND_HELPER", helper)
        .env("CDXM_BASH", bash)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let scripts = root.join("agmsg-scripts");
    if scripts.is_dir() {
        command.env("AGMSG_SCRIPTS_DIR", scripts);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn installed_foreground_helper_accepts_native_owner_pid() {
    let binary = env!("CARGO_BIN_EXE_codex-monitor");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let cwd = root.canonicalize().unwrap();
    let enable = Command::new(binary)
        .args([
            "app-hook",
            "enable",
            "--team",
            "cdxm",
            "--name",
            "codex",
            "--session",
            "native-owner",
            "--cwd",
            cwd.to_str().unwrap(),
        ])
        .env("CDXM_APP_HOOK_ROOT", root)
        .output()
        .unwrap();
    assert!(enable.status.success());

    let scripts = root.join("agmsg-scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::write(
        scripts.join("inbox.sh"),
        "#!/usr/bin/env bash\nprintf '1 new message(s):\\n\\n  [now] alice: native owner live\\n\\n'\n",
    )
    .unwrap();
    let helper = repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh");
    let payload = json!({
        "session_id": "native-owner",
        "cwd": cwd,
        "stop_hook_active": false
    });
    let output = stop_hook(binary, root, &helper, &bash_path(), &payload);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["decision"], "block");
    assert!(result["reason"]
        .as_str()
        .unwrap()
        .contains("native owner live"));
}

#[test]
fn app_hook_cli_enables_continues_rearms_and_disables() {
    let binary = env!("CARGO_BIN_EXE_codex-monitor");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let cwd = root.canonicalize().unwrap();
    let hooks_path = root.join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().unwrap()).unwrap();
    fs::write(
        &hooks_path,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#,
    )
    .unwrap();

    let enable = Command::new(binary)
        .args([
            "app-hook",
            "enable",
            "--team",
            "cdxm",
            "--name",
            "codex",
            "--session",
            "session-one",
            "--cwd",
            cwd.to_str().unwrap(),
        ])
        .env("CDXM_APP_HOOK_ROOT", root)
        .output()
        .unwrap();
    assert!(
        enable.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&enable.stderr)
    );
    assert!(String::from_utf8_lossy(&enable.stdout).contains("trust-required"));
    let hooks: Value = serde_json::from_slice(&fs::read(&hooks_path).unwrap()).unwrap();
    let hooks_text = hooks.to_string();
    assert!(hooks_text.contains("other"));
    assert!(hooks_text.contains("__app-stop-hook"));
    assert!(root
        .join(".codex-monitor/app-hooks/session-one.json")
        .is_file());

    let helper = root.join("fake-foreground.sh");
    fs::write(
        &helper,
        "#!/usr/bin/env bash\nprintf '1 new message(s):\\n\\n  [now] alice: hello\\n\\n'\n",
    )
    .unwrap();
    let bash = bash_path();
    for active in [false, true] {
        let payload = json!({
            "session_id": "session-one",
            "cwd": cwd,
            "turn_id": "turn-one",
            "stop_hook_active": active
        });
        let output = stop_hook(binary, root, &helper, &bash, &payload);
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["decision"], "block");
        assert!(result["reason"].as_str().unwrap().contains("Sender: alice"));
    }

    let disable = Command::new(binary)
        .args(["app-hook", "disable", "--session", "session-one"])
        .env("CDXM_APP_HOOK_ROOT", root)
        .output()
        .unwrap();
    assert!(disable.status.success());
    assert!(!root
        .join(".codex-monitor/app-hooks/session-one.json")
        .exists());

    let payload = json!({
        "session_id": "session-one",
        "cwd": cwd,
        "turn_id": "turn-two",
        "stop_hook_active": false
    });
    let output = stop_hook(
        binary,
        root,
        &helper,
        Path::new("definitely-missing-bash"),
        &payload,
    );
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result, json!({ "continue": true }));
}
