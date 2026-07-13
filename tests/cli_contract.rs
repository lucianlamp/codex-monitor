use std::{fs, path::Path, process::Command};

#[test]
fn package_exposes_one_native_binary() {
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"codex-monitor\""));
    assert!(!manifest.contains("name = \"cdxm\""));
    assert!(!manifest.contains("name = \"cdxm-codex-app-bridge\""));
}

#[test]
fn package_exposes_codex_monitor_binary() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");

    let primary_output = Command::new(primary).arg("--help").output().unwrap();

    assert!(primary_output.status.success());

    let primary_help = String::from_utf8(primary_output.stdout).unwrap();

    assert!(primary_help.contains("codex-monitor"));
    assert!(primary_help.contains("threads"));
    assert!(primary_help.contains("targets"));
    assert!(primary_help.contains("send"));
    assert!(primary_help.contains("agmsg"));
    assert!(primary_help.contains("monitor"));
    assert!(primary_help.contains("remote"));
    assert!(!primary_help.contains("steer"));
    assert!(!primary_help.contains("loaded"));
    assert!(primary_help.contains("[default: auto]"));
}

#[test]
fn unix_installer_uses_single_native_binary_and_cdxm_launcher() {
    let installer =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")).unwrap();
    assert!(installer.contains("tar -xzf \"$dl_dir/$archive\" -C \"$extract_dir\" codex-monitor"));
    assert!(!installer.contains("codex-monitor cdxm"));
    assert!(installer.contains("cargo install --path \"$SOURCE_DIR\" --bin codex-monitor"));
    assert!(installer.contains("exec \"$SCRIPT_DIR/codex-monitor\" \"$@\""));
    assert!(installer.contains("__finalize-macos-install"));
}

#[test]
fn docs_define_macos_update_and_single_binary_migration() {
    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    assert!(readme.contains("macOS arm64 and Intel"));
    assert!(readme.contains("one native executable"));
    assert!(readme.contains("reloads each exact LaunchAgent"));
    assert!(readme.contains("Linux self-update is not supported"));
}

#[test]
fn update_command_is_public_and_apply_worker_is_hidden() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let help = Command::new(primary).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("update"));
    assert!(!help.contains("__apply-update"));

    let update = Command::new(primary)
        .args(["update", "--help"])
        .output()
        .unwrap();
    assert!(update.status.success());
}

#[test]
fn app_hook_commands_are_public_and_handler_is_hidden() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let help = Command::new(primary).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("app-hook"));
    assert!(!help.contains("__app-stop-hook"));

    let nested = Command::new(primary)
        .args(["app-hook", "--help"])
        .output()
        .unwrap();
    assert!(nested.status.success());
    let nested = String::from_utf8(nested.stdout).unwrap();
    for command in ["enable", "disable", "status"] {
        assert!(
            nested.contains(command),
            "missing app-hook command `{command}`"
        );
    }

    let hook_root = tempfile::tempdir().unwrap();
    let hidden = Command::new(primary)
        .args(["__app-stop-hook", "--help"])
        .env("CDXM_APP_HOOK_ROOT", hook_root.path())
        .output()
        .unwrap();
    assert!(hidden.status.success());
}

#[test]
fn internal_macos_finalizer_is_hidden_but_registered() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let help = Command::new(primary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(!String::from_utf8(help.stdout)
        .unwrap()
        .contains("__finalize-macos-install"));

    let finalizer = Command::new(primary)
        .args(["__finalize-macos-install", "--help"])
        .output()
        .unwrap();
    assert!(finalizer.status.success());
}

#[test]
fn client_info_name_contract_is_fixed() {
    assert_eq!(codex_monitor::CLIENT_INFO_NAME, "codex-monitor");
    assert_eq!(codex_monitor::CLIENT_INFO_TITLE, "Codex Monitor");
}

#[test]
fn agmsg_watch_can_target_cwd_without_explicit_thread() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["agmsg", "watch", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--cwd <CWD>"));
    assert!(help.contains("--thread <THREAD>"));
    assert!(help.contains("--mode <MODE>"));
    assert!(help.contains("--dry-run"));
}

#[test]
fn agmsg_help_exposes_doctor() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["agmsg", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("watch"));
    assert!(help.contains("doctor"));
    assert!(help.contains("launch-agent"));
}

#[test]
fn agmsg_launch_agent_commands_are_exposed() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["agmsg", "launch-agent", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("print"));
    assert!(help.contains("install"));
    assert!(help.contains("status"));
    assert!(help.contains("uninstall"));
}

#[test]
fn agmsg_launch_agent_print_exposes_codex_monitor_path_option() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["agmsg", "launch-agent", "print", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--codex-monitor-path"));
}

#[test]
fn monitor_help_exposes_watch() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["monitor", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("watch"));
}

#[test]
fn monitor_watch_exposes_agmsg_adapter() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["monitor", "watch", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("agmsg"));
}

#[test]
fn monitor_watch_agmsg_can_target_cwd_without_explicit_thread() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["monitor", "watch", "agmsg", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--cwd <CWD>"));
    assert!(help.contains("--thread <THREAD>"));
    assert!(help.contains("--mode <MODE>"));
    assert!(help.contains("--dry-run"));
    assert!(help.contains("--agmsg-db <AGMSG_DB>"));
}

#[test]
fn remote_help_focuses_on_doctor_as_primary_surface() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["remote", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("doctor"));
    assert!(help.contains("connect"));
    assert!(!help.contains("enable"));
    assert!(!help.contains("disable"));
    assert!(!help.contains("clients"));
    assert!(!help.contains("claim"));
    assert!(!help.contains("\n  monitor"));
}

#[test]
fn remote_connect_is_visible_as_controller_probe() {
    let alias = env!("CARGO_BIN_EXE_codex-monitor");
    let output = Command::new(alias)
        .args(["remote", "connect", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--max-messages"));
    assert!(help.contains("--timeout-ms"));
    assert!(help.contains("--client-id"));
}
