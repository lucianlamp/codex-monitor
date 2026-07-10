use std::{fs, path::Path};

fn workflow() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
    fs::read_to_string(p).expect("release.yml should exist")
}

#[test]
fn release_workflow_triggers_on_version_tags() {
    let wf = workflow();
    assert!(wf.contains("tags:"));
    assert!(wf.contains("v*"));
}

#[test]
fn release_workflow_builds_all_three_targets() {
    let wf = workflow();
    assert!(wf.contains("aarch64-apple-darwin"));
    assert!(wf.contains("x86_64-apple-darwin"));
    assert!(wf.contains("x86_64-pc-windows-msvc"));
}

#[test]
fn release_workflow_uses_available_intel_macos_runner() {
    let wf = workflow();
    assert!(wf.contains("macos-26-intel"));
    assert!(!wf.contains("macos-13"));
}

#[test]
fn release_workflow_packages_fixed_asset_names_with_checksums() {
    let wf = workflow();
    assert!(wf.contains("codex-monitor-${{ matrix.target }}.tar.gz"));
    assert!(wf.contains("codex-monitor-${{ matrix.target }}.zip"));
    assert!(wf.contains(".sha256"));
    // builds both binaries
    assert!(wf.contains("--bins") || (wf.contains("codex-monitor") && wf.contains("cdxm")));
    // attaches to a GitHub release
    assert!(wf.contains("softprops/action-gh-release"));
}

#[test]
fn windows_release_packages_app_bridge() {
    let wf = workflow();
    assert!(wf.contains("release/cdxm-codex-app-bridge.exe"));
    assert!(
        wf.contains("Copy-Item \"target/${{ matrix.target }}/release/cdxm-codex-app-bridge.exe\"")
    );
}
