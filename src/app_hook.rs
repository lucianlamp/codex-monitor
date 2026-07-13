use anyhow::{bail, Context};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncReadExt, process::Command};

pub const APP_HOOK_STATUS_MESSAGE: &str = "Waiting for agmsg via codex-monitor";
const APP_HOOK_TIMEOUT_SECONDS: u64 = 86_400;
const MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AppHookPaths {
    pub hooks_json: PathBuf,
    pub markers_dir: PathBuf,
}

impl AppHookPaths {
    fn from_root(root: &Path) -> Self {
        Self {
            hooks_json: root.join(".codex/hooks.json"),
            markers_dir: root.join(".codex-monitor/app-hooks"),
        }
    }

    #[cfg(test)]
    fn for_test(root: &Path) -> Self {
        Self::from_root(root)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HookChange {
    Added,
    Updated,
    Unchanged,
}

impl HookChange {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct AppHookMarker {
    pub version: u32,
    pub session_id: String,
    pub team: String,
    pub name: String,
    pub cwd: PathBuf,
    pub updated_at: String,
}

impl AppHookMarker {
    pub fn new(
        session_id: String,
        team: String,
        name: String,
        cwd: PathBuf,
    ) -> anyhow::Result<Self> {
        validate_session_id(&session_id)?;
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_secs()
            .to_string();
        Ok(Self {
            version: MARKER_VERSION,
            session_id,
            team,
            name,
            cwd,
            updated_at,
        })
    }
}

pub fn default_paths() -> anyhow::Result<AppHookPaths> {
    if let Some(root) = std::env::var_os("CDXM_APP_HOOK_ROOT") {
        return Ok(AppHookPaths::from_root(&PathBuf::from(root)));
    }
    let base = BaseDirs::new().context("could not resolve the user home directory")?;
    Ok(AppHookPaths::from_root(base.home_dir()))
}

pub fn ensure_hook_installed(
    paths: &AppHookPaths,
    executable: &Path,
) -> anyhow::Result<HookChange> {
    let desired = owned_handler(executable);
    let mut root = match fs::read(&paths.hooks_json) {
        Ok(raw) => serde_json::from_slice::<Value>(&raw).with_context(|| {
            format!(
                "existing Codex hook configuration is not valid JSON: {}",
                paths.hooks_json.display()
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({ "hooks": {} }),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read Codex hooks from {}",
                    paths.hooks_json.display()
                )
            })
        }
    };

    let root_object = root
        .as_object_mut()
        .context("Codex hooks root must be a JSON object")?;
    let hooks = root_object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Codex hooks field must be a JSON object")?;
    let stop_groups = hooks
        .entry("Stop")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("Codex Stop hooks field must be a JSON array")?;

    let mut owned_positions = Vec::new();
    for (group_index, group) in stop_groups.iter().enumerate() {
        let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
            continue;
        };
        for (handler_index, handler) in handlers.iter().enumerate() {
            if is_owned_handler(handler) {
                owned_positions.push((group_index, handler_index));
            }
        }
    }

    if owned_positions.len() == 1 {
        let (group_index, handler_index) = owned_positions[0];
        if stop_groups[group_index]["hooks"][handler_index] == desired {
            return Ok(HookChange::Unchanged);
        }
    }

    let change = if owned_positions.is_empty() {
        HookChange::Added
    } else {
        HookChange::Updated
    };

    for group in stop_groups.iter_mut() {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            handlers.retain(|handler| !is_owned_handler(handler));
        }
    }
    stop_groups.push(json!({ "hooks": [desired] }));

    let encoded = serde_json::to_vec_pretty(&root).context("failed to encode Codex hooks")?;
    atomic_write(&paths.hooks_json, &encoded)?;
    Ok(change)
}

pub fn hook_is_installed(paths: &AppHookPaths) -> anyhow::Result<bool> {
    let raw = match fs::read(&paths.hooks_json) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read Codex hooks from {}",
                    paths.hooks_json.display()
                )
            })
        }
    };
    let root: Value = serde_json::from_slice(&raw).with_context(|| {
        format!(
            "existing Codex hook configuration is not valid JSON: {}",
            paths.hooks_json.display()
        )
    })?;
    Ok(root
        .get("hooks")
        .and_then(|hooks| hooks.get("Stop"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .any(is_owned_handler))
}

pub async fn run_stop_hook_from_stdio() -> anyhow::Result<i32> {
    let mut raw = String::new();
    tokio::io::stdin()
        .read_to_string(&mut raw)
        .await
        .context("failed to read Codex Stop hook input")?;
    let input: StopHookInput =
        serde_json::from_str(&raw).context("Codex Stop hook input is not valid JSON")?;
    let paths = default_paths()?;
    let Some(marker) = matching_marker(&paths, &input)? else {
        println!("{}", json!({ "continue": true }));
        return Ok(0);
    };
    let bash = resolve_bash()?;
    let helper = foreground_helper_path()?;
    let output = run_active_stop_hook(marker, &bash, &helper).await?;
    println!(
        "{}",
        serde_json::to_string(&output).context("failed to encode Stop hook output")?
    );
    Ok(0)
}

#[derive(Debug, Clone, Deserialize)]
struct StopHookInput {
    session_id: String,
    cwd: PathBuf,
    #[serde(default)]
    stop_hook_active: bool,
}

#[cfg(test)]
async fn run_stop_hook_with_paths(
    paths: &AppHookPaths,
    input: StopHookInput,
    bash: &Path,
    helper: &Path,
) -> anyhow::Result<Value> {
    let Some(marker) = matching_marker(paths, &input)? else {
        return Ok(json!({ "continue": true }));
    };
    run_active_stop_hook(marker, bash, helper).await
}

fn matching_marker(
    paths: &AppHookPaths,
    input: &StopHookInput,
) -> anyhow::Result<Option<AppHookMarker>> {
    validate_session_id(&input.session_id)?;
    let _already_continued = input.stop_hook_active;
    let marker = match load_marker(paths, &input.session_id) {
        Ok(Some(marker)) => marker,
        Ok(None) => return Ok(None),
        Err(error) => {
            eprintln!("codex-monitor App Stop hook ignored invalid marker: {error:#}");
            return Ok(None);
        }
    };
    let input_cwd = input
        .cwd
        .canonicalize()
        .unwrap_or_else(|_| input.cwd.clone());
    if input_cwd != marker.cwd {
        return Ok(None);
    }
    Ok(Some(marker))
}

async fn run_active_stop_hook(
    marker: AppHookMarker,
    bash: &Path,
    helper: &Path,
) -> anyhow::Result<Value> {
    let helper_arg = helper_path_for_bash(helper);
    let mut command = Command::new(bash);
    command
        .arg(helper_arg)
        .arg(&marker.team)
        .arg(&marker.name)
        .kill_on_drop(true);
    #[cfg(not(windows))]
    command.env("CDXM_FOREGROUND_PARENT_PID", std::process::id().to_string());
    let output = command.output().await.with_context(|| {
        format!(
            "failed to run App foreground helper {} through {}",
            helper.display(),
            bash.display()
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!(
            "App foreground helper failed with {}: {}",
            output.status,
            stderr
        );
    }
    let stdout =
        String::from_utf8(output.stdout).context("App foreground helper output was not UTF-8")?;
    let reason = format_inbox_events(&marker.team, &marker.name, &stdout)?;
    Ok(json!({ "decision": "block", "reason": reason }))
}

fn format_inbox_events(team: &str, name: &str, output: &str) -> anyhow::Result<String> {
    let mut events = Vec::new();
    for line in output.replace('\r', "").lines() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let (_, payload) = line
            .split_once("] ")
            .context("agmsg inbox row is missing its timestamp delimiter")?;
        let (sender, body) = payload
            .split_once(": ")
            .context("agmsg inbox row is missing its sender delimiter")?;
        let body = body.replace("\\n", "\n").replace("\\t", "\t");
        events.push(format!(
            "agmsg monitor event\n\nTeam: {team}\nRecipient: {name}\nSender: {sender}\n\n{body}\n\nIf this requires a reply, use the agmsg scripts rather than answering only in chat."
        ));
    }
    if events.is_empty() {
        bail!("App foreground helper returned no parseable agmsg messages");
    }
    Ok(events.join("\n\n"))
}

fn foreground_helper_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CDXM_APP_HOOK_FOREGROUND_HELPER") {
        return Ok(PathBuf::from(path));
    }
    let base = BaseDirs::new().context("could not resolve the user home directory")?;
    Ok(base
        .home_dir()
        .join(".codex/skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh"))
}

fn resolve_bash() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CDXM_BASH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!("CDXM_BASH is not an executable file: {}", path.display());
    }

    #[cfg(windows)]
    {
        let mut candidates = vec![
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"),
        ];
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local_app_data).join("Programs/Git/bin/bash.exe"));
        }
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Ok(path);
        }
        bail!("Git Bash is required for the Codex App Stop hook");
    }

    #[cfg(not(windows))]
    {
        let path = PathBuf::from("/bin/bash");
        if path.is_file() {
            return Ok(path);
        }
        bail!("/bin/bash is required for the Codex App Stop hook");
    }
}

fn helper_path_for_bash(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
}

pub fn enable_marker(paths: &AppHookPaths, marker: &AppHookMarker) -> anyhow::Result<()> {
    validate_marker(marker)?;
    let encoded = serde_json::to_vec_pretty(marker).context("failed to encode App hook marker")?;
    atomic_write(&marker_path(paths, &marker.session_id)?, &encoded)
}

pub fn load_marker(
    paths: &AppHookPaths,
    session_id: &str,
) -> anyhow::Result<Option<AppHookMarker>> {
    let path = marker_path(paths, session_id)?;
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read App hook marker {}", path.display()))
        }
    };
    let marker: AppHookMarker = serde_json::from_slice(&raw)
        .with_context(|| format!("App hook marker is not valid JSON: {}", path.display()))?;
    validate_marker(&marker)?;
    if marker.session_id != session_id {
        bail!("App hook marker session id does not match its filename");
    }
    Ok(Some(marker))
}

pub fn disable_marker(paths: &AppHookPaths, session_id: &str) -> anyhow::Result<bool> {
    let path = marker_path(paths, session_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove App hook marker {}", path.display())),
    }
}

fn owned_handler(executable: &Path) -> Value {
    let command = format!("\"{}\" __app-stop-hook", executable.display());
    json!({
        "type": "command",
        "command": command,
        "commandWindows": command,
        "timeout": APP_HOOK_TIMEOUT_SECONDS,
        "statusMessage": APP_HOOK_STATUS_MESSAGE,
    })
}

fn is_owned_handler(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("command")
        && value.get("statusMessage").and_then(Value::as_str) == Some(APP_HOOK_STATUS_MESSAGE)
}

fn marker_path(paths: &AppHookPaths, session_id: &str) -> anyhow::Result<PathBuf> {
    validate_session_id(session_id)?;
    Ok(paths.markers_dir.join(format!("{session_id}.json")))
}

fn validate_marker(marker: &AppHookMarker) -> anyhow::Result<()> {
    if marker.version != MARKER_VERSION {
        bail!("unsupported App hook marker version: {}", marker.version);
    }
    validate_session_id(&marker.session_id)?;
    if marker.team.trim().is_empty() || marker.name.trim().is_empty() {
        bail!("App hook marker team and name must be non-empty");
    }
    if !marker.cwd.is_absolute() {
        bail!("App hook marker cwd must be absolute");
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> anyhow::Result<()> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("invalid Codex session id");
    }
    Ok(())
}

fn atomic_write(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    let temporary = parent.join(format!(".codex-monitor-{}-{nanos}.tmp", std::process::id()));
    fs::write(&temporary, contents)
        .with_context(|| format!("failed to write temporary file {}", temporary.display()))?;
    if let Err(error) = publish_atomic(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to publish {}", path.display()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_atomic(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(windows)]
fn publish_atomic(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{fs, path::Path};

    fn write_helper(path: &Path, body: &str) {
        fs::write(path, format!("#!/usr/bin/env bash\n{body}\n")).unwrap();
    }

    fn marker(session_id: &str, cwd: &Path) -> AppHookMarker {
        AppHookMarker {
            version: 1,
            session_id: session_id.to_owned(),
            team: "cdxm".into(),
            name: "codex".into(),
            cwd: cwd.to_path_buf(),
            updated_at: "123".into(),
        }
    }

    #[test]
    fn install_preserves_other_hooks_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        fs::create_dir_all(paths.hooks_json.parent().unwrap()).unwrap();
        fs::write(
            &paths.hooks_json,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other","statusMessage":"other"}]}]}}"#,
        )
        .unwrap();

        assert_eq!(
            ensure_hook_installed(&paths, Path::new("/opt/codex-monitor")).unwrap(),
            HookChange::Added
        );
        let once = fs::read(&paths.hooks_json).unwrap();
        let json: Value = serde_json::from_slice(&once).unwrap();
        let handlers = json["hooks"]["Stop"].as_array().unwrap();
        assert!(handlers.iter().any(|group| group["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|handler| handler["command"] == "other")));

        assert_eq!(
            ensure_hook_installed(&paths, Path::new("/opt/codex-monitor")).unwrap(),
            HookChange::Unchanged
        );
        assert_eq!(fs::read(&paths.hooks_json).unwrap(), once);
    }

    #[test]
    fn install_updates_only_the_owned_handler() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        fs::create_dir_all(paths.hooks_json.parent().unwrap()).unwrap();
        fs::write(
            &paths.hooks_json,
            format!(
                r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"other"}},{{"type":"command","command":"old __app-stop-hook","statusMessage":"{}"}}]}}]}}}}"#,
                APP_HOOK_STATUS_MESSAGE
            ),
        )
        .unwrap();

        assert_eq!(
            ensure_hook_installed(&paths, Path::new("/new/codex-monitor")).unwrap(),
            HookChange::Updated
        );
        let raw = fs::read_to_string(&paths.hooks_json).unwrap();
        assert!(raw.contains("other"));
        assert!(!raw.contains("old __app-stop-hook"));
        assert!(raw.contains("/new/codex-monitor"));
    }

    #[test]
    fn invalid_hook_json_is_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        fs::create_dir_all(paths.hooks_json.parent().unwrap()).unwrap();
        fs::write(&paths.hooks_json, b"not-json").unwrap();

        assert!(ensure_hook_installed(&paths, Path::new("/opt/codex-monitor")).is_err());
        assert_eq!(fs::read(&paths.hooks_json).unwrap(), b"not-json");
    }

    #[test]
    fn marker_round_trips_and_disable_is_session_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        let cwd = temp.path().canonicalize().unwrap();
        let first = marker("session-one", &cwd);
        let second = marker("session-two", &cwd);

        enable_marker(&paths, &first).unwrap();
        enable_marker(&paths, &second).unwrap();
        assert_eq!(load_marker(&paths, "session-one").unwrap(), Some(first));
        assert!(disable_marker(&paths, "session-one").unwrap());
        assert_eq!(load_marker(&paths, "session-one").unwrap(), None);
        assert_eq!(load_marker(&paths, "session-two").unwrap(), Some(second));
        assert!(!disable_marker(&paths, "session-one").unwrap());
    }

    #[test]
    fn marker_rejects_unsafe_session_ids() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        let cwd = temp.path().canonicalize().unwrap();

        for session_id in ["", "../escape", "with/slash", "with\\slash"] {
            assert!(enable_marker(&paths, &marker(session_id, &cwd)).is_err());
            assert!(load_marker(&paths, session_id).is_err());
        }
    }

    #[tokio::test]
    async fn stop_hook_without_marker_returns_immediately() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        let helper = temp.path().join("must-not-run.sh");
        write_helper(&helper, "exit 99");
        let input = StopHookInput {
            session_id: "missing-session".into(),
            cwd: temp.path().canonicalize().unwrap(),
            stop_hook_active: false,
        };

        let output = run_stop_hook_with_paths(&paths, input, &test_bash(), &helper)
            .await
            .unwrap();
        assert_eq!(output, json!({ "continue": true }));
    }

    #[tokio::test]
    async fn stop_hook_cwd_mismatch_does_not_run_helper() {
        let temp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        enable_marker(
            &paths,
            &marker("session-one", &temp.path().canonicalize().unwrap()),
        )
        .unwrap();
        let helper = temp.path().join("must-not-run.sh");
        write_helper(&helper, "exit 99");
        let input = StopHookInput {
            session_id: "session-one".into(),
            cwd: other.path().canonicalize().unwrap(),
            stop_hook_active: false,
        };

        let output = run_stop_hook_with_paths(&paths, input, &test_bash(), &helper)
            .await
            .unwrap();
        assert_eq!(output, json!({ "continue": true }));
    }

    #[tokio::test]
    async fn stop_hook_ignores_a_malformed_or_unsupported_marker() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        fs::create_dir_all(&paths.markers_dir).unwrap();
        let path = paths.markers_dir.join("session-one.json");
        let helper = temp.path().join("must-not-run.sh");
        write_helper(&helper, "exit 99");
        let input = StopHookInput {
            session_id: "session-one".into(),
            cwd: temp.path().canonicalize().unwrap(),
            stop_hook_active: false,
        };

        for raw in ["not-json", r#"{"version":999}"#] {
            fs::write(&path, raw).unwrap();
            let output = run_stop_hook_with_paths(&paths, input.clone(), &test_bash(), &helper)
                .await
                .unwrap();
            assert_eq!(output, json!({ "continue": true }));
        }
    }

    #[tokio::test]
    async fn stop_hook_formats_messages_and_rearms_after_continuation() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        let cwd = temp.path().canonicalize().unwrap();
        enable_marker(&paths, &marker("session-one", &cwd)).unwrap();
        let helper = temp.path().join("messages.sh");
        write_helper(
            &helper,
            "printf '2 new message(s):\\n\\n  [now] alice: first\\n  [later] bob: second\\\\nline\\n\\n'",
        );

        for stop_hook_active in [false, true] {
            let input = StopHookInput {
                session_id: "session-one".into(),
                cwd: cwd.clone(),
                stop_hook_active,
            };
            let output = run_stop_hook_with_paths(&paths, input, &test_bash(), &helper)
                .await
                .unwrap();
            assert_eq!(output["decision"], "block");
            let reason = output["reason"].as_str().unwrap();
            assert!(reason.contains("Sender: alice\n\nfirst"));
            assert!(reason.contains("Sender: bob\n\nsecond\nline"));
            assert!(reason.contains("use the agmsg scripts"));
        }
    }

    #[tokio::test]
    async fn stop_hook_propagates_helper_failure_without_json() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppHookPaths::for_test(temp.path());
        let cwd = temp.path().canonicalize().unwrap();
        enable_marker(&paths, &marker("session-one", &cwd)).unwrap();
        let helper = temp.path().join("failure.sh");
        write_helper(&helper, "printf 'helper failed' >&2; exit 23");
        let input = StopHookInput {
            session_id: "session-one".into(),
            cwd,
            stop_hook_active: false,
        };

        let error = run_stop_hook_with_paths(&paths, input, &test_bash(), &helper)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("helper failed"));
    }

    fn test_bash() -> PathBuf {
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
            panic!("Git Bash is required for App hook tests");
        }
        #[cfg(not(windows))]
        {
            PathBuf::from("/bin/bash")
        }
    }
}
