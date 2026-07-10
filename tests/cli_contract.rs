use std::process::Command;

#[test]
fn package_exposes_codex_monitor_binaries() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let short_alias = env!("CARGO_BIN_EXE_cdxm");

    let primary_output = Command::new(primary).arg("--help").output().unwrap();
    let short_alias_output = Command::new(short_alias).arg("--help").output().unwrap();

    assert!(primary_output.status.success());
    assert!(short_alias_output.status.success());

    let primary_help = String::from_utf8(primary_output.stdout).unwrap();
    let short_alias_help = String::from_utf8(short_alias_output.stdout).unwrap();

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
    assert!(short_alias_help.contains("codex-monitor"));
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
fn client_info_name_contract_is_fixed() {
    assert_eq!(codex_monitor::CLIENT_INFO_NAME, "codex-monitor");
    assert_eq!(codex_monitor::CLIENT_INFO_TITLE, "Codex Monitor");
}

#[test]
fn agmsg_watch_can_target_cwd_without_explicit_thread() {
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
    assert!(!help.contains("monitor"));
}

#[test]
fn remote_connect_is_visible_as_controller_probe() {
    let alias = env!("CARGO_BIN_EXE_cdxm");
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
