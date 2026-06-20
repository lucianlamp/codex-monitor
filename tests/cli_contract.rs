use std::process::Command;

#[test]
fn package_exposes_primary_and_alias_binaries() {
    let primary = env!("CARGO_BIN_EXE_codex-control-bridge");
    let alias = env!("CARGO_BIN_EXE_ccb");

    let primary_output = Command::new(primary).arg("--help").output().unwrap();
    let alias_output = Command::new(alias).arg("--help").output().unwrap();

    assert!(primary_output.status.success());
    assert!(alias_output.status.success());

    let primary_help = String::from_utf8(primary_output.stdout).unwrap();
    let alias_help = String::from_utf8(alias_output.stdout).unwrap();

    assert!(primary_help.contains("codex-control-bridge"));
    assert!(primary_help.contains("threads"));
    assert!(primary_help.contains("send"));
    assert!(primary_help.contains("agmsg"));
    assert!(alias_help.contains("codex-control-bridge"));
}

#[test]
fn client_info_name_contract_is_fixed() {
    assert_eq!(
        codex_control_bridge::CLIENT_INFO_NAME,
        "codex-control-bridge"
    );
    assert_eq!(
        codex_control_bridge::CLIENT_INFO_TITLE,
        "Codex Control Bridge"
    );
}
