use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

fn bash_executable() -> PathBuf {
    #[cfg(windows)]
    {
        let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            return git_bash;
        }
    }
    PathBuf::from("bash")
}

fn bash_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy().replace('\\', "/");
        let bytes = raw.as_bytes();
        if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
            return format!("/{}/{}", raw[..1].to_ascii_lowercase(), &raw[3..]);
        }
        raw
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}

fn write_fake_codex(path: &Path, label: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("#!/usr/bin/env bash\nprintf '%s\\n' '{label}'\n"),
    )
    .unwrap();
    let status = Command::new(bash_executable())
        .args(["-c", &format!("chmod +x '{}'", bash_path(path))])
        .status()
        .unwrap();
    assert!(status.success());
}

fn run_shim_output(path_entries: &[&Path], local_app_data: &Path) -> Output {
    let shim =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/codex-monitor/scripts/codex-shim.sh");
    let mut path = path_entries
        .iter()
        .map(|entry| bash_path(entry))
        .collect::<Vec<_>>();
    path.extend(["/usr/bin".into(), "/bin".into()]);
    Command::new(bash_executable())
        .arg(bash_path(&shim))
        .arg("--version")
        .env("PATH", path.join(":"))
        .env("LOCALAPPDATA", bash_path(local_app_data))
        .env_remove("CODEX_MONITOR_REAL_CODEX")
        .output()
        .unwrap()
}

fn run_shim(path_entries: &[&Path], local_app_data: &Path) -> String {
    let output = run_shim_output(path_entries, local_app_data);
    assert!(
        output.status.success(),
        "shim failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn shim_prefers_package_manager_codex_over_desktop_bundle() {
    let temp = TempDir::new().unwrap();
    let local_app_data = temp.path().join("LocalAppData");
    let desktop_dir = local_app_data.join("OpenAI/Codex/bin");
    let package_dir = temp.path().join("npm");
    write_fake_codex(&desktop_dir.join("codex"), "desktop-old");
    write_fake_codex(&package_dir.join("codex"), "package-current");

    assert_eq!(
        run_shim(&[&desktop_dir, &package_dir], &local_app_data),
        "package-current"
    );
}

#[test]
fn shim_rejects_desktop_bundle_as_the_only_real_codex() {
    let temp = TempDir::new().unwrap();
    let local_app_data = temp.path().join("LocalAppData");
    let desktop_dir = local_app_data.join("OpenAI/Codex/bin");
    write_fake_codex(&desktop_dir.join("codex"), "desktop-old");

    let output = run_shim_output(&[&desktop_dir], &local_app_data);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("refusing Windows Desktop Codex fallback")
    );
}
