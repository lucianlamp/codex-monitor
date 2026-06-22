use std::path::PathBuf;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgmsgLaunchAgentConfig {
    pub team: String,
    pub name: String,
    pub thread: Option<String>,
    pub cwd: PathBuf,
    pub mode: crate::cli::SendMode,
    pub codex_monitor_path: PathBuf,
    pub endpoint: crate::target::Endpoint,
    pub agmsg_db: Option<PathBuf>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LaunchAgentStatus {
    pub label: String,
    pub plist_path: PathBuf,
    pub installed: bool,
    pub loaded: Option<bool>,
    pub stdout_log: LaunchAgentLogStatus,
    pub stderr_log: LaunchAgentLogStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LaunchAgentLogStatus {
    pub path: PathBuf,
    pub modified_unix_ms: Option<u128>,
    pub len: Option<u64>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LaunchAgentInstallResult {
    pub label: String,
    pub plist_path: PathBuf,
    pub loaded: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgmsgLaunchAgentIdentity {
    pub team: String,
    pub name: String,
}

pub fn label_for_agmsg_watch(config: &AgmsgLaunchAgentConfig) -> String {
    format!(
        "com.local.codex-monitor.agmsg.{}.{}",
        sanitize_label_segment(&config.team),
        sanitize_label_segment(&config.name)
    )
}

pub fn parse_agmsg_launch_agent_label(label: &str) -> Option<AgmsgLaunchAgentIdentity> {
    let rest = label.strip_prefix("com.local.codex-monitor.agmsg.")?;
    let (team, name) = rest.split_once('.')?;
    if team.is_empty() || name.is_empty() {
        return None;
    }
    Some(AgmsgLaunchAgentIdentity {
        team: team.to_string(),
        name: name.to_string(),
    })
}

pub fn render_agmsg_watch_plist(config: &AgmsgLaunchAgentConfig) -> anyhow::Result<String> {
    let label = label_for_agmsg_watch(config);
    let stdout_path = log_path(&label, "out.log")?;
    let stderr_path = log_path(&label, "err.log")?;
    let args = program_arguments(config);
    let mut plist = String::new();
    plist.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    plist.push('\n');
    plist.push_str(
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#,
    );
    plist.push('\n');
    plist.push_str("<plist version=\"1.0\">\n<dict>\n");
    push_key_string(&mut plist, "Label", &label);
    plist.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for arg in args {
        plist.push_str("    <string>");
        plist.push_str(&escape_xml(&arg));
        plist.push_str("</string>\n");
    }
    plist.push_str("  </array>\n");
    push_key_string(
        &mut plist,
        "WorkingDirectory",
        &config.cwd.to_string_lossy(),
    );
    plist.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    plist.push_str("  <key>KeepAlive</key>\n  <true/>\n");
    push_key_string(
        &mut plist,
        "StandardOutPath",
        &stdout_path.to_string_lossy(),
    );
    push_key_string(
        &mut plist,
        "StandardErrorPath",
        &stderr_path.to_string_lossy(),
    );
    plist.push_str("</dict>\n</plist>\n");
    Ok(plist)
}

pub fn default_codex_monitor_path() -> anyhow::Result<PathBuf> {
    std::env::current_exe().map_err(Into::into)
}

pub fn plist_path_for_label(label: &str) -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

pub fn install_agmsg_watch_agent(
    config: &AgmsgLaunchAgentConfig,
    force: bool,
    load: bool,
) -> anyhow::Result<LaunchAgentInstallResult> {
    require_macos()?;
    let label = label_for_agmsg_watch(config);
    let plist_path = plist_path_for_label(&label)?;
    if plist_path.exists() && !force {
        anyhow::bail!(
            "launch agent already exists: {}; pass --force to overwrite",
            plist_path.display()
        );
    }
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(log_dir()?)?;
    std::fs::write(&plist_path, render_agmsg_watch_plist(config)?)?;
    if load {
        bootstrap_agent(&plist_path)?;
    }
    Ok(LaunchAgentInstallResult {
        label,
        plist_path,
        loaded: load,
    })
}

pub fn status_for_agmsg_watch_agent(team: &str, name: &str) -> anyhow::Result<LaunchAgentStatus> {
    require_macos()?;
    let config = AgmsgLaunchAgentConfig {
        team: team.to_string(),
        name: name.to_string(),
        thread: None,
        cwd: std::env::current_dir()?,
        mode: crate::cli::SendMode::Auto,
        codex_monitor_path: default_codex_monitor_path()?,
        endpoint: crate::target::Endpoint::Auto,
        agmsg_db: None,
    };
    let label = label_for_agmsg_watch(&config);
    let plist_path = plist_path_for_label(&label)?;
    let stdout_log = log_status_for_path(log_path(&label, "out.log")?);
    let stderr_log = log_status_for_path(log_path(&label, "err.log")?);
    let installed = plist_path.exists();
    let service = format!("gui/{}/{}", user_id()?, label);
    let output = std::process::Command::new("launchctl")
        .args(["print", &service])
        .output();
    let (loaded, detail) = match output {
        Ok(output) if output.status.success() => {
            (Some(true), String::from_utf8_lossy(&output.stdout).into())
        }
        Ok(output) => (Some(false), String::from_utf8_lossy(&output.stderr).into()),
        Err(error) => (None, error.to_string()),
    };
    Ok(LaunchAgentStatus {
        label,
        plist_path,
        installed,
        loaded,
        stdout_log,
        stderr_log,
        detail,
    })
}

pub fn statuses_for_agmsg_team(team: &str) -> anyhow::Result<Vec<LaunchAgentStatus>> {
    require_macos()?;
    let sanitized_team = sanitize_label_segment(team);
    let prefix = format!("com.local.codex-monitor.agmsg.{sanitized_team}.");
    let dir = home_dir()?.join("Library/LaunchAgents");
    let mut statuses = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(statuses),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(label) = file_name.strip_suffix(".plist") else {
            continue;
        };
        if !label.starts_with(&prefix) {
            continue;
        }
        if let Some(identity) = parse_agmsg_launch_agent_label(label) {
            statuses.push(status_for_agmsg_watch_agent(
                &identity.team,
                &identity.name,
            )?);
        }
    }
    statuses.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(statuses)
}

pub fn uninstall_agmsg_watch_agent(team: &str, name: &str) -> anyhow::Result<LaunchAgentStatus> {
    require_macos()?;
    let status = status_for_agmsg_watch_agent(team, name)?;
    if status.loaded == Some(true) {
        let service = format!("gui/{}/{}", user_id()?, status.label);
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &service])
            .status();
    }
    if status.plist_path.exists() {
        std::fs::remove_file(&status.plist_path)?;
    }
    Ok(LaunchAgentStatus {
        installed: false,
        loaded: Some(false),
        ..status
    })
}

pub fn log_status_for_path(path: PathBuf) -> LaunchAgentLogStatus {
    match std::fs::metadata(&path) {
        Ok(metadata) => LaunchAgentLogStatus {
            path,
            modified_unix_ms: metadata.modified().ok().and_then(system_time_unix_ms),
            len: Some(metadata.len()),
        },
        Err(_) => LaunchAgentLogStatus {
            path,
            modified_unix_ms: None,
            len: None,
        },
    }
}

fn system_time_unix_ms(time: std::time::SystemTime) -> Option<u128> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

fn program_arguments(config: &AgmsgLaunchAgentConfig) -> Vec<String> {
    let mut args = vec![config.codex_monitor_path.to_string_lossy().to_string()];
    match &config.endpoint {
        crate::target::Endpoint::Auto => {}
        crate::target::Endpoint::Managed => args.extend(["--target".into(), "managed".into()]),
        crate::target::Endpoint::App => args.extend(["--target".into(), "app".into()]),
        crate::target::Endpoint::Explicit(endpoint) => {
            args.extend(["--endpoint".into(), endpoint.clone()])
        }
    }
    args.extend([
        "agmsg".into(),
        "watch".into(),
        "--team".into(),
        config.team.clone(),
        "--name".into(),
        config.name.clone(),
    ]);
    if let Some(thread) = &config.thread {
        args.extend(["--thread".into(), thread.clone()]);
    }
    if config.mode != crate::cli::SendMode::Auto {
        args.extend(["--mode".into(), send_mode_arg(config.mode).into()]);
    }
    args.extend(["--cwd".into(), config.cwd.to_string_lossy().to_string()]);
    if let Some(agmsg_db) = &config.agmsg_db {
        args.extend(["--agmsg-db".into(), agmsg_db.to_string_lossy().to_string()]);
    }
    args
}

fn send_mode_arg(mode: crate::cli::SendMode) -> &'static str {
    match mode {
        crate::cli::SendMode::Auto => "auto",
        crate::cli::SendMode::Start => "start",
        crate::cli::SendMode::Steer => "steer",
    }
}

fn bootstrap_agent(plist_path: &std::path::Path) -> anyhow::Result<()> {
    let domain = format!("gui/{}", user_id()?);
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", &domain, &plist_path.to_string_lossy()])
        .status()?;
    if !status.success() {
        anyhow::bail!("launchctl bootstrap failed for {}", plist_path.display());
    }
    Ok(())
}

fn user_id() -> anyhow::Result<String> {
    let output = std::process::Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        anyhow::bail!("id -u failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sanitize_label_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "x".to_string()
    } else {
        sanitized
    }
}

fn push_key_string(plist: &mut String, key: &str, value: &str) {
    plist.push_str("  <key>");
    plist.push_str(&escape_xml(key));
    plist.push_str("</key>\n  <string>");
    plist.push_str(&escape_xml(value));
    plist.push_str("</string>\n");
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn log_path(label: &str, suffix: &str) -> anyhow::Result<PathBuf> {
    Ok(log_dir()?.join(format!("{label}.{suffix}")))
}

fn log_dir() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join("Library/Logs/codex-monitor"))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn require_macos() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        anyhow::bail!("launch-agent commands are only available on macOS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agmsg_launch_agent_label_is_stable_for_team_and_name() {
        let config = AgmsgLaunchAgentConfig {
            team: "dev".into(),
            name: "kimura".into(),
            thread: None,
            cwd: PathBuf::from("/Users/ysk411/dev/codex-monitor"),
            mode: crate::cli::SendMode::Auto,
            codex_monitor_path: PathBuf::from("/Users/ysk411/.cargo/bin/cdxm"),
            endpoint: crate::target::Endpoint::Auto,
            agmsg_db: None,
        };

        assert_eq!(
            label_for_agmsg_watch(&config),
            "com.local.codex-monitor.agmsg.dev.kimura"
        );
    }

    #[test]
    fn agmsg_launch_agent_plist_runs_cwd_watch_with_explicit_endpoint() {
        let config = AgmsgLaunchAgentConfig {
            team: "dev".into(),
            name: "kimura".into(),
            thread: Some("thread-1".into()),
            cwd: PathBuf::from("/Users/ysk411/dev/codex-monitor"),
            mode: crate::cli::SendMode::Start,
            codex_monitor_path: PathBuf::from("/Users/ysk411/.cargo/bin/cdxm"),
            endpoint: crate::target::Endpoint::Explicit("unix:///tmp/app.sock".into()),
            agmsg_db: Some(PathBuf::from("/tmp/messages.db")),
        };

        let plist = render_agmsg_watch_plist(&config).unwrap();
        assert!(plist.contains("<string>com.local.codex-monitor.agmsg.dev.kimura</string>"));
        assert!(plist.contains("<string>/Users/ysk411/.cargo/bin/cdxm</string>"));
        assert!(plist.contains("<string>--endpoint</string>"));
        assert!(plist.contains("<string>unix:///tmp/app.sock</string>"));
        assert!(plist.contains("<string>agmsg</string>"));
        assert!(plist.contains("<string>watch</string>"));
        assert!(plist.contains("<string>--thread</string>"));
        assert!(plist.contains("<string>thread-1</string>"));
        assert!(plist.contains("<string>--mode</string>"));
        assert!(plist.contains("<string>start</string>"));
        assert!(plist.contains("<string>--cwd</string>"));
        assert!(plist.contains("<string>/Users/ysk411/dev/codex-monitor</string>"));
        assert!(plist.contains("<string>--agmsg-db</string>"));
        assert!(plist.contains("<string>/tmp/messages.db</string>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn log_status_reports_missing_and_existing_file_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("agent.err.log");

        let missing = log_status_for_path(log_path.clone());
        assert_eq!(missing.path, log_path);
        assert_eq!(missing.modified_unix_ms, None);
        assert_eq!(missing.len, None);

        std::fs::write(&missing.path, "old error\n").unwrap();
        let existing = log_status_for_path(missing.path.clone());
        assert_eq!(existing.path, missing.path);
        assert!(existing.modified_unix_ms.is_some());
        assert_eq!(existing.len, Some(10));
    }

    #[test]
    fn parses_team_and_name_from_agmsg_launch_agent_label() {
        let parsed =
            parse_agmsg_launch_agent_label("com.local.codex-monitor.agmsg.emeria.steve").unwrap();

        assert_eq!(parsed.team, "emeria");
        assert_eq!(parsed.name, "steve");
        assert!(parse_agmsg_launch_agent_label("com.example.other").is_none());
    }
}
