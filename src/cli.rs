use crate::client::AppServerClient;
use crate::remote_control::{
    RemoteControlBackendClient, RemoteControlClient, RemoteControlPairingStart,
    RemoteControlPairingStatus, RemoteControlStatus,
};
use crate::target::endpoint_from_options;
use crate::transport::open_endpoint_transport;
use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{future::Future, path::PathBuf, time::Duration};

const AUTO_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Parser)]
#[command(name = "codex-monitor")]
#[command(about = "codex-monitor: local-first monitor for Codex app-server control plane events.")]
pub struct Cli {
    #[arg(long, global = true)]
    pub endpoint: Option<String>,

    #[arg(long, global = true, value_enum, default_value_t = TargetKind::Auto)]
    pub target: TargetKind,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum TargetKind {
    Auto,
    Managed,
    App,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Targets,
    Threads {
        #[arg(long)]
        cwd: String,
    },
    #[command(hide = true)]
    Loaded,
    Send {
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        text: String,
        #[arg(long, value_enum, default_value_t = SendMode::Auto)]
        mode: SendMode,
        #[arg(long)]
        turn: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    #[command(hide = true)]
    Steer {
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        text: String,
        #[arg(long)]
        turn: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Agmsg {
        #[command(subcommand)]
        command: AgmsgCommand,
    },
    Monitor {
        #[command(subcommand)]
        command: MonitorCommand,
    },
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum SendMode {
    Auto,
    Start,
    Steer,
}

impl SendMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SendMode::Auto => "auto",
            SendMode::Start => "start",
            SendMode::Steer => "steer",
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgmsgCommand {
    Watch {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = SendMode::Auto)]
        mode: SendMode,
        #[arg(long)]
        agmsg_db: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Doctor {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = SendMode::Auto)]
        mode: SendMode,
        #[arg(long)]
        agmsg_db: Option<String>,
    },
    LaunchAgent {
        #[command(subcommand)]
        command: AgmsgLaunchAgentCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgmsgLaunchAgentCommand {
    Print(AgmsgLaunchAgentOptions),
    Install {
        #[command(flatten)]
        options: AgmsgLaunchAgentOptions,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        load: bool,
    },
    Status {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
    },
    Uninstall {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum MonitorCommand {
    Watch {
        #[command(subcommand)]
        adapter: MonitorWatchCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum MonitorWatchCommand {
    Agmsg {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = SendMode::Auto)]
        mode: SendMode,
        #[arg(long)]
        agmsg_db: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct AgmsgLaunchAgentOptions {
    #[arg(long)]
    team: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    thread: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = SendMode::Auto)]
    mode: SendMode,
    #[arg(long)]
    agmsg_db: Option<PathBuf>,
    #[arg(long)]
    codex_monitor_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RemoteCommand {
    #[command(hide = true)]
    Status,
    #[command(hide = true)]
    Enable,
    #[command(hide = true)]
    Disable,
    #[command(hide = true)]
    PairStart {
        #[arg(long)]
        manual_code: bool,
    },
    #[command(hide = true)]
    PairStatus {
        #[arg(long)]
        pairing_code: Option<String>,
        #[arg(long)]
        manual_pairing_code: Option<String>,
    },
    #[command(hide = true)]
    Clients {
        #[arg(long)]
        environment_id: Option<String>,
    },
    #[command(hide = true)]
    Monitor {
        #[arg(long)]
        count: Option<u64>,
        #[arg(long, default_value_t = 2000)]
        interval_ms: u64,
    },
    Doctor {
        #[arg(long)]
        environment_id: Option<String>,
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
        #[arg(long)]
        auth_file: Option<PathBuf>,
        #[arg(long)]
        global_state_file: Option<PathBuf>,
        #[arg(long)]
        device_key_module: Option<PathBuf>,
        #[arg(long)]
        skip_refresh: bool,
        #[arg(long)]
        skip_backend: bool,
        #[arg(long)]
        skip_device_key: bool,
    },
    #[command(hide = true)]
    Claim {
        #[arg(long, alias = "manual-code")]
        manual_pairing_code: String,
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
        #[arg(long)]
        auth_file: Option<PathBuf>,
        #[arg(long)]
        global_state_file: Option<PathBuf>,
        #[arg(long)]
        skip_refresh: bool,
    },
    /// Connect as an enrolled remote-control client and print observed messages.
    Connect {
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        api_base_url: Option<String>,
        #[arg(long)]
        websocket_url: Option<String>,
        #[arg(long)]
        auth_file: Option<PathBuf>,
        #[arg(long)]
        global_state_file: Option<PathBuf>,
        #[arg(long)]
        device_key_module: Option<PathBuf>,
        #[arg(long)]
        skip_refresh: bool,
        #[arg(long, default_value_t = 10000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 0)]
        max_messages: usize,
    },
}

#[derive(Debug, Clone)]
struct RemoteDoctorOptions {
    environment_id: Option<String>,
    client_id: Option<String>,
    api_base_url: Option<String>,
    auth_file: Option<PathBuf>,
    global_state_file: Option<PathBuf>,
    device_key_module: Option<PathBuf>,
    skip_refresh: bool,
    skip_backend: bool,
    skip_device_key: bool,
}

pub async fn run_from_env() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    run(cli).await
}

pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    let requested_endpoint = endpoint_from_options(cli.endpoint.clone(), cli.target);
    match cli.command {
        Commands::Targets => {
            for candidate in crate::target::discover_auto_endpoint_candidates() {
                println!(
                    "{}\t{}",
                    candidate.source,
                    crate::target::endpoint_label(&candidate.endpoint)
                );
            }
            Ok(0)
        }
        Commands::Threads { cwd } => {
            let threads = resolve_threads_for_cwd(requested_endpoint, &cwd).await?;
            for thread in threads {
                println!(
                    "{}\t{}\t{}",
                    thread.id,
                    thread.title.unwrap_or_else(|| "-".into()),
                    thread.cwd.unwrap_or_else(|| "-".into())
                );
            }
            Ok(0)
        }
        Commands::Loaded => {
            let endpoint = crate::target::resolve_default_auto_endpoint(requested_endpoint)?;
            let transport = open_endpoint_transport(endpoint).await?;
            let mut client = AppServerClient::new(transport);
            let operation = async {
                client.initialize().await?;
                client.thread_loaded_list().await
            }
            .await;
            let close_result = client.close().await;
            let loaded_threads = operation?;
            close_result?;
            for thread_id in loaded_threads {
                println!("{thread_id}");
            }
            Ok(0)
        }
        Commands::Send {
            thread,
            cwd,
            text,
            mode,
            turn,
            wait,
        } => run_send_command(requested_endpoint, thread, cwd, text, mode, turn, wait).await,
        Commands::Steer {
            thread,
            cwd,
            text,
            turn,
            wait,
        } => {
            run_send_command(
                requested_endpoint,
                thread,
                cwd,
                text,
                SendMode::Steer,
                turn,
                wait,
            )
            .await
        }
        Commands::Agmsg { command } => match command {
            AgmsgCommand::Watch {
                team,
                name,
                thread,
                cwd,
                mode,
                agmsg_db,
                dry_run,
            } => {
                crate::delivery::run_agmsg_watch(crate::delivery::AgmsgWatchOptions {
                    endpoint: requested_endpoint,
                    team,
                    name,
                    thread,
                    cwd,
                    mode,
                    agmsg_db,
                    dry_run,
                })
                .await
            }
            AgmsgCommand::Doctor {
                team,
                name,
                thread,
                cwd,
                mode,
                agmsg_db,
            } => {
                run_agmsg_doctor(requested_endpoint, team, name, thread, cwd, mode, agmsg_db).await
            }
            AgmsgCommand::LaunchAgent { command } => {
                run_agmsg_launch_agent_command(requested_endpoint, command).await
            }
        },
        Commands::Monitor { command } => match command {
            MonitorCommand::Watch { adapter } => match adapter {
                MonitorWatchCommand::Agmsg {
                    team,
                    name,
                    thread,
                    cwd,
                    mode,
                    agmsg_db,
                    dry_run,
                } => {
                    crate::delivery::run_agmsg_watch(crate::delivery::AgmsgWatchOptions {
                        endpoint: requested_endpoint,
                        team,
                        name,
                        thread,
                        cwd,
                        mode,
                        agmsg_db,
                        dry_run,
                    })
                    .await
                }
            },
        },
        Commands::Remote { command } => {
            let endpoint = crate::target::resolve_default_auto_endpoint(requested_endpoint)?;
            run_remote_command(endpoint, command).await
        }
    }
}

async fn run_send_command(
    endpoint: crate::target::Endpoint,
    thread: Option<String>,
    cwd: Option<PathBuf>,
    text: String,
    mode: SendMode,
    turn: Option<String>,
    wait: bool,
) -> anyhow::Result<i32> {
    let (endpoint, thread) = resolve_endpoint_and_thread(endpoint, thread, cwd).await?;
    let transport = open_endpoint_transport(endpoint).await?;
    let mut client = AppServerClient::new(transport);
    let operation = async {
        client.initialize().await?;
        client.ensure_thread_loaded(&thread).await?;
        match mode {
            SendMode::Start => {
                if wait {
                    client.turn_start_and_wait(&thread, &text).await?;
                } else {
                    client.turn_start(&thread, &text).await?;
                }
            }
            SendMode::Steer => {
                let turn = match turn {
                    Some(turn) => turn,
                    None => client.active_turn_id(&thread).await?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "thread {thread} has no active turn; pass --turn or use --mode start"
                        )
                    })?,
                };
                if wait {
                    client.turn_steer_and_wait(&thread, &turn, &text).await?;
                } else {
                    client.turn_steer(&thread, &turn, &text).await?;
                }
            }
            SendMode::Auto => {
                if wait {
                    let active_turn = match turn {
                        Some(turn) => Some(turn),
                        None => client.active_turn_id(&thread).await?,
                    };
                    if let Some(active_turn) = active_turn {
                        let steered = client
                            .turn_steer_and_wait(&thread, &active_turn, &text)
                            .await;
                        if steered.is_ok() {
                            return anyhow::Ok(());
                        }
                    }
                    client.turn_start_and_wait(&thread, &text).await?;
                } else {
                    client.turn_start_or_steer(&thread, &text, turn).await?;
                }
            }
        }
        anyhow::Ok(())
    }
    .await;
    let close_result = client.close().await;
    operation?;
    close_result?;
    Ok(0)
}

async fn run_agmsg_launch_agent_command(
    endpoint: crate::target::Endpoint,
    command: AgmsgLaunchAgentCommand,
) -> anyhow::Result<i32> {
    match command {
        AgmsgLaunchAgentCommand::Print(options) => {
            let config = agmsg_launch_agent_config(endpoint, options)?;
            print!("{}", crate::launchd::render_agmsg_watch_plist(&config)?);
            Ok(0)
        }
        AgmsgLaunchAgentCommand::Install {
            options,
            force,
            load,
        } => {
            let config = agmsg_launch_agent_config(endpoint, options)?;
            let result = crate::launchd::install_agmsg_watch_agent(&config, force, load)?;
            println!(
                "installed\t{}\t{}\tloaded={}",
                result.label,
                result.plist_path.display(),
                result.loaded
            );
            Ok(0)
        }
        AgmsgLaunchAgentCommand::Status { team, name } => {
            let status = crate::launchd::status_for_agmsg_watch_agent(&team, &name)?;
            print_launch_agent_status(&status);
            Ok(0)
        }
        AgmsgLaunchAgentCommand::Uninstall { team, name } => {
            let status = crate::launchd::uninstall_agmsg_watch_agent(&team, &name)?;
            println!(
                "uninstalled\t{}\t{}",
                status.label,
                status.plist_path.display()
            );
            Ok(0)
        }
    }
}

fn agmsg_launch_agent_config(
    endpoint: crate::target::Endpoint,
    options: AgmsgLaunchAgentOptions,
) -> anyhow::Result<crate::launchd::AgmsgLaunchAgentConfig> {
    Ok(crate::launchd::AgmsgLaunchAgentConfig {
        team: options.team,
        name: options.name,
        thread: options.thread,
        cwd: options.cwd.unwrap_or(std::env::current_dir()?),
        mode: options.mode,
        codex_monitor_path: options
            .codex_monitor_path
            .unwrap_or(crate::launchd::default_codex_monitor_path()?),
        endpoint,
        agmsg_db: options.agmsg_db,
    })
}

fn print_launch_agent_status(status: &crate::launchd::LaunchAgentStatus) {
    let loaded = match status.loaded {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    };
    println!(
        "status\t{}\tinstalled={}\tloaded={}\targs_match={}\tdesired_thread={}\tactive_thread={}\tplist={}\tstdout_log={}\tstdout_mtime={}\tstderr_log={}\tstderr_mtime={}",
        status.label,
        status.installed,
        loaded,
        format_optional_bool(status.arguments_match),
        format_optional_str(status.desired_thread.as_deref()),
        format_optional_str(status.active_thread.as_deref()),
        status.plist_path.display(),
        status.stdout_log.path.display(),
        format_optional_u128(status.stdout_log.modified_unix_ms),
        status.stderr_log.path.display(),
        format_optional_u128(status.stderr_log.modified_unix_ms)
    );
}

async fn run_agmsg_doctor(
    endpoint: crate::target::Endpoint,
    team: String,
    name: String,
    thread: Option<String>,
    cwd: Option<PathBuf>,
    mode: SendMode,
    agmsg_db: Option<String>,
) -> anyhow::Result<i32> {
    let cwd = cwd.unwrap_or(std::env::current_dir()?);
    let cwd_text = cwd.to_string_lossy().to_string();
    let state_key = format!("agmsg:{team}:{name}");
    let db_path = agmsg_db
        .map(PathBuf::from)
        .unwrap_or_else(crate::sources::agmsg::AgmsgSource::default_db_path);
    let state_path = crate::delivery::default_state_path()?;

    print_agmsg_doctor_row(&[
        "doctor",
        "input",
        "ok",
        &format!("team={team}"),
        &format!("name={name}"),
        &format!("cwd={cwd_text}"),
        &format!("mode={}", mode.as_str()),
    ]);

    for candidate in crate::target::discover_auto_endpoint_candidates() {
        print_agmsg_doctor_row(&[
            "doctor",
            "target",
            "candidate",
            &format!("source={}", candidate.source),
            &format!(
                "endpoint={}",
                crate::target::endpoint_label(&candidate.endpoint)
            ),
        ]);
    }

    match thread.as_deref() {
        Some(thread) => match resolve_endpoint_for_loaded_thread(endpoint.clone(), thread).await {
            Ok(resolved) => print_agmsg_doctor_row(&[
                "doctor",
                "thread",
                "ok",
                &format!("id={thread}"),
                &format!("endpoint={}", crate::target::endpoint_label(&resolved)),
            ]),
            Err(error) => print_agmsg_doctor_row(&[
                "doctor",
                "thread",
                "error",
                &format!("id={thread}"),
                &error.to_string(),
            ]),
        },
        None => {
            for row in agmsg_doctor_thread_rows(endpoint.clone(), &cwd_text).await {
                print_agmsg_doctor_row(&row.iter().map(String::as_str).collect::<Vec<_>>());
            }
        }
    }

    let store = crate::state::StateStore::new(state_path);
    let state = match store.load().await {
        Ok(state) => {
            print_agmsg_doctor_row(&[
                "doctor",
                "state",
                "ok",
                &format!("key={state_key}"),
                &format!("last_seen={}", state.last_seen(&state_key)),
                &format!("path={}", store.path().display()),
            ]);
            state
        }
        Err(error) => {
            print_agmsg_doctor_row(&[
                "doctor",
                "state",
                "error",
                &format!("key={state_key}"),
                &format!("path={}", store.path().display()),
                &error.to_string(),
            ]);
            crate::state::State::default()
        }
    };

    let source =
        crate::sources::agmsg::AgmsgSource::new(db_path.clone(), team.clone(), name.clone());
    match source.inbox_stats(state.last_seen(&state_key)) {
        Ok(stats) => print_agmsg_doctor_row(&[
            "doctor",
            "inbox",
            "ok",
            &format!("db={}", db_path.display()),
            &format!("latest_id={}", format_optional_u64(stats.latest_id)),
            &format!(
                "latest_unread_id={}",
                format_optional_u64(stats.latest_unread_id)
            ),
            &format!(
                "next_pending_after_state_id={}",
                format_optional_u64(stats.next_pending_after_state_id)
            ),
            &format!(
                "pending_after_state_count={}",
                stats.pending_after_state_count
            ),
            &format!("unread_count={}", stats.unread_count),
        ]),
        Err(error) => print_agmsg_doctor_row(&[
            "doctor",
            "inbox",
            "error",
            &format!("db={}", db_path.display()),
            &error.to_string(),
        ]),
    }

    match crate::launchd::status_for_agmsg_watch_agent(&team, &name) {
        Ok(status) => {
            let loaded = match status.loaded {
                Some(true) => "true",
                Some(false) => "false",
                None => "unknown",
            };
            print_agmsg_doctor_row(&[
                "doctor",
                "launch-agent",
                "ok",
                &format!("label={}", status.label),
                &format!("installed={}", status.installed),
                &format!("loaded={loaded}"),
                &format!(
                    "args_match={}",
                    format_optional_bool(status.arguments_match)
                ),
                &format!(
                    "desired_thread={}",
                    format_optional_str(status.desired_thread.as_deref())
                ),
                &format!(
                    "active_thread={}",
                    format_optional_str(status.active_thread.as_deref())
                ),
                &format!("plist={}", status.plist_path.display()),
                &format!("stdout_log={}", status.stdout_log.path.display()),
                &format!(
                    "stdout_mtime={}",
                    format_optional_u128(status.stdout_log.modified_unix_ms)
                ),
                &format!("stderr_log={}", status.stderr_log.path.display()),
                &format!(
                    "stderr_mtime={}",
                    format_optional_u128(status.stderr_log.modified_unix_ms)
                ),
            ]);
        }
        Err(error) => print_agmsg_doctor_row(&[
            "doctor",
            "launch-agent",
            "warn",
            &format!("team={team}"),
            &format!("name={name}"),
            &error.to_string(),
        ]),
    }

    match crate::launchd::statuses_for_agmsg_team(&team) {
        Ok(statuses) => {
            for status in statuses {
                let relevance = if status.label
                    == crate::launchd::label_for_agmsg_watch(
                        &crate::launchd::AgmsgLaunchAgentConfig {
                            team: team.clone(),
                            name: name.clone(),
                            thread: None,
                            cwd: PathBuf::from(&cwd_text),
                            mode,
                            codex_monitor_path: PathBuf::new(),
                            endpoint: crate::target::Endpoint::Auto,
                            agmsg_db: None,
                        },
                    ) {
                    "target"
                } else {
                    "same-team"
                };
                print_agmsg_doctor_row(&[
                    "doctor",
                    "launch-agent-team",
                    relevance,
                    &format!("label={}", status.label),
                    &format!("installed={}", status.installed),
                    &format!(
                        "args_match={}",
                        format_optional_bool(status.arguments_match)
                    ),
                    &format!(
                        "desired_thread={}",
                        format_optional_str(status.desired_thread.as_deref())
                    ),
                    &format!(
                        "active_thread={}",
                        format_optional_str(status.active_thread.as_deref())
                    ),
                    &format!("plist={}", status.plist_path.display()),
                    &format!(
                        "stdout_mtime={}",
                        format_optional_u128(status.stdout_log.modified_unix_ms)
                    ),
                    &format!(
                        "stderr_mtime={}",
                        format_optional_u128(status.stderr_log.modified_unix_ms)
                    ),
                ]);
            }
        }
        Err(error) => print_agmsg_doctor_row(&[
            "doctor",
            "launch-agent-team",
            "warn",
            &format!("team={team}"),
            &error.to_string(),
        ]),
    }

    for consumer in discover_agmsg_consumer_processes() {
        let relevance = if consumer.team.as_deref() == Some(team.as_str())
            && consumer.name.as_deref() == Some(name.as_str())
        {
            "target"
        } else if consumer.team.as_deref() == Some(team.as_str()) {
            "same-team"
        } else {
            "potential"
        };
        let thread_match = thread
            .as_deref()
            .map(|desired| consumer.thread.as_deref() == Some(desired));
        print_agmsg_doctor_row(&[
            "doctor",
            "consumer",
            relevance,
            &format!("pid={}", consumer.pid),
            &format!("kind={}", consumer.kind),
            &format!("team={}", consumer.team.as_deref().unwrap_or("-")),
            &format!("name={}", consumer.name.as_deref().unwrap_or("-")),
            &format!("thread={}", format_optional_str(consumer.thread.as_deref())),
            &format!("desired_thread={}", format_optional_str(thread.as_deref())),
            &format!("thread_match={}", format_optional_bool(thread_match)),
            &format!("command={}", consumer.command),
        ]);
    }

    print_agmsg_doctor_row(&[
        "doctor",
        "note",
        "ack-vs-visible",
        "app-server ack and codex-monitor state advancement do not prove the event was visible in the current Codex UI",
    ]);

    Ok(0)
}

async fn agmsg_doctor_thread_rows(
    endpoint: crate::target::Endpoint,
    cwd: &str,
) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    match endpoint {
        crate::target::Endpoint::Auto => {
            for candidate in crate::target::discover_auto_endpoint_candidates() {
                match probe_loaded_threads_for_cwd(
                    candidate.endpoint.clone(),
                    cwd,
                    AUTO_PROBE_TIMEOUT,
                )
                .await
                {
                    Ok(threads) if threads.is_empty() => rows.push(vec![
                        "doctor".into(),
                        "thread".into(),
                        "none".into(),
                        format!(
                            "endpoint={}",
                            crate::target::endpoint_label(&candidate.endpoint)
                        ),
                        format!("source={}", candidate.source),
                    ]),
                    Ok(threads) => {
                        for thread in threads {
                            rows.push(vec![
                                "doctor".into(),
                                "thread".into(),
                                "ok".into(),
                                format!(
                                    "endpoint={}",
                                    crate::target::endpoint_label(&candidate.endpoint)
                                ),
                                format!("source={}", candidate.source),
                                format!("id={}", thread.id),
                                format!("title={}", thread.title.unwrap_or_else(|| "-".into())),
                                format!("cwd={}", thread.cwd.unwrap_or_else(|| "-".into())),
                            ]);
                        }
                    }
                    Err(error) => rows.push(vec![
                        "doctor".into(),
                        "thread".into(),
                        "error".into(),
                        format!(
                            "endpoint={}",
                            crate::target::endpoint_label(&candidate.endpoint)
                        ),
                        format!("source={}", candidate.source),
                        error.to_string(),
                    ]),
                }
            }
        }
        endpoint => match loaded_threads_for_cwd(endpoint.clone(), cwd).await {
            Ok(threads) if threads.is_empty() => rows.push(vec![
                "doctor".into(),
                "thread".into(),
                "none".into(),
                format!("endpoint={}", crate::target::endpoint_label(&endpoint)),
            ]),
            Ok(threads) => {
                for thread in threads {
                    rows.push(vec![
                        "doctor".into(),
                        "thread".into(),
                        "ok".into(),
                        format!("endpoint={}", crate::target::endpoint_label(&endpoint)),
                        format!("id={}", thread.id),
                        format!("title={}", thread.title.unwrap_or_else(|| "-".into())),
                        format!("cwd={}", thread.cwd.unwrap_or_else(|| "-".into())),
                    ]);
                }
            }
            Err(error) => rows.push(vec![
                "doctor".into(),
                "thread".into(),
                "error".into(),
                format!("endpoint={}", crate::target::endpoint_label(&endpoint)),
                error.to_string(),
            ]),
        },
    }
    rows
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct AgmsgConsumerProcess {
    pid: u32,
    kind: String,
    team: Option<String>,
    name: Option<String>,
    thread: Option<String>,
    command: String,
}

fn discover_agmsg_consumer_processes() -> Vec<AgmsgConsumerProcess> {
    #[cfg(windows)]
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$ErrorActionPreference='SilentlyContinue'; Get-CimInstance Win32_Process | Where-Object { $_.CommandLine } | ForEach-Object { \"{0} {1}\" -f $_.ProcessId, $_.CommandLine }",
        ])
        .output();

    #[cfg(not(windows))]
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            parse_agmsg_consumer_processes(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

fn parse_agmsg_consumer_processes(text: &str) -> Vec<AgmsgConsumerProcess> {
    text.lines()
        .filter_map(parse_agmsg_consumer_process)
        .collect()
}

fn parse_agmsg_consumer_process(line: &str) -> Option<AgmsgConsumerProcess> {
    let trimmed = line.trim();
    let (pid, command) = trimmed.split_once(char::is_whitespace)?;
    let pid = pid.parse::<u32>().ok()?;
    let command = command.trim();
    let tokens = command.split_whitespace().collect::<Vec<_>>();

    let is_agmsg_watch = tokens.windows(2).any(|window| window == ["agmsg", "watch"])
        || tokens
            .windows(3)
            .any(|window| window == ["monitor", "watch", "agmsg"]);

    let kind = if is_agmsg_watch && command_invokes_codex_monitor(&tokens) {
        "codex-monitor-agmsg-watch"
    } else if command.contains("codex-bridge") {
        "codex-bridge"
    } else if command.contains("watch.sh") {
        "agmsg-watch-sh"
    } else {
        return None;
    };

    let team = option_from_cli_tokens(&tokens, "--team");
    let mut name = option_from_cli_tokens(&tokens, "--name");
    let thread = option_from_cli_tokens(&tokens, "--thread");
    if name.is_none() && kind == "agmsg-watch-sh" {
        name = tokens.last().map(|value| (*value).to_string());
    }

    Some(AgmsgConsumerProcess {
        pid,
        kind: kind.to_string(),
        team,
        name,
        thread,
        command: command.to_string(),
    })
}

fn command_invokes_codex_monitor(tokens: &[&str]) -> bool {
    tokens.iter().any(|token| {
        let basename = token
            .trim_matches('"')
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(token);
        matches!(
            basename,
            "cdxm" | "cdxm.exe" | "codex-monitor" | "codex-monitor.exe"
        )
    })
}

fn option_from_cli_tokens(tokens: &[&str], flag: &str) -> Option<String> {
    for (index, token) in tokens.iter().enumerate() {
        if *token == flag {
            return tokens.get(index + 1).map(|value| (*value).to_string());
        }
        if let Some(value) = token.strip_prefix(&format!("{flag}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn print_agmsg_doctor_row(fields: &[&str]) {
    let row = fields
        .iter()
        .map(|field| sanitize_tsv_field(field))
        .collect::<Vec<_>>();
    println!("{}", row.join("\t"));
}

fn sanitize_tsv_field(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_u128(value: Option<u128>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "-",
    }
}

fn format_optional_str(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

pub(crate) async fn resolve_endpoint_and_thread(
    endpoint: crate::target::Endpoint,
    thread: Option<String>,
    cwd: Option<PathBuf>,
) -> anyhow::Result<(crate::target::Endpoint, String)> {
    let endpoint = crate::target::resolve_app_endpoint(endpoint)?;
    if let Some(thread) = thread {
        return Ok((
            resolve_endpoint_for_loaded_thread(endpoint, &thread).await?,
            thread,
        ));
    }

    let cwd = cwd.unwrap_or(std::env::current_dir()?);
    let cwd = cwd.to_string_lossy().to_string();
    match endpoint {
        crate::target::Endpoint::Auto => {
            let candidates = crate::target::discover_auto_endpoint_candidates();
            let mut matches = Vec::new();
            let mut failures = Vec::new();
            for candidate in candidates {
                match probe_loaded_thread_for_cwd(
                    candidate.endpoint.clone(),
                    &cwd,
                    AUTO_PROBE_TIMEOUT,
                )
                .await
                {
                    Ok(Some(thread)) => matches.push((candidate, thread)),
                    Ok(None) => {}
                    Err(error) => failures.push(format!(
                        "{}: {error:#}",
                        crate::target::endpoint_label(&candidate.endpoint)
                    )),
                }
            }
            match matches.as_slice() {
                [(candidate, thread)] => Ok((candidate.endpoint.clone(), thread.clone())),
                [] => {
                    let detail = if failures.is_empty() {
                        "no candidate reported a loaded thread".to_string()
                    } else {
                        format!("live probe failures: {}", failures.join("; "))
                    };
                    anyhow::bail!(
                        "could not auto-resolve a loaded thread for cwd {cwd}; open the target thread in an app-server-bound Codex session, pass --endpoint, or pass --thread with a loaded endpoint ({detail})"
                    )
                }
                many => {
                    let choices = many
                        .iter()
                        .map(|(candidate, thread)| {
                            format!(
                                "{} thread={} ({})",
                                crate::target::endpoint_label(&candidate.endpoint),
                                thread,
                                candidate.source
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "multiple auto endpoints have loaded threads for cwd {cwd}; pass --endpoint and/or --thread explicitly: {choices}"
                    )
                }
            }
        }
        crate::target::Endpoint::Managed => {
            let thread = loaded_thread_for_cwd(crate::target::Endpoint::Managed, &cwd)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "managed endpoint has no loaded thread for cwd {cwd}; launch Codex through a remote/app-server shim or pass a loaded --endpoint"
                    )
                })?;
            Ok((crate::target::Endpoint::Managed, thread))
        }
        live_endpoint => {
            let thread = loaded_thread_for_cwd(live_endpoint.clone(), &cwd).await?.ok_or_else(|| {
                anyhow::anyhow!(
                    "endpoint {} has no loaded thread for cwd {cwd}; open the target thread or pass --thread",
                    crate::target::endpoint_label(&live_endpoint)
                )
            })?;
            Ok((live_endpoint, thread))
        }
    }
}

async fn resolve_threads_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
) -> anyhow::Result<Vec<crate::target::ThreadSummary>> {
    let endpoint = crate::target::resolve_app_endpoint(endpoint)?;
    if endpoint != crate::target::Endpoint::Auto {
        return threads_for_cwd(endpoint, cwd).await;
    }

    let candidates = crate::target::discover_auto_endpoint_candidates();
    resolve_threads_for_cwd_from_candidates(candidates, cwd, AUTO_PROBE_TIMEOUT).await
}

async fn resolve_threads_for_cwd_from_candidates(
    candidates: Vec<crate::target::EndpointCandidate>,
    cwd: &str,
    probe_timeout: Duration,
) -> anyhow::Result<Vec<crate::target::ThreadSummary>> {
    let mut matches = Vec::new();
    let mut failures = Vec::new();
    for candidate in candidates {
        match probe_loaded_threads_for_cwd(candidate.endpoint.clone(), cwd, probe_timeout).await {
            Ok(threads) if !threads.is_empty() => matches.push((candidate, threads)),
            Ok(_) => {}
            Err(error) => failures.push(format!(
                "{}: {error:#}",
                crate::target::endpoint_label(&candidate.endpoint)
            )),
        }
    }

    match matches.as_slice() {
        [(_, threads)] => Ok(threads.clone()),
        [] => match threads_for_cwd(crate::target::Endpoint::Managed, cwd).await {
            Ok(threads) => Ok(threads),
            Err(error) => {
                let detail = if failures.is_empty() {
                    error.to_string()
                } else {
                    format!("{error}; live probe failures: {}", failures.join("; "))
                };
                anyhow::bail!(
                    "could not auto-resolve a thread source for cwd {cwd}; pass --endpoint or --target explicitly ({detail})"
                )
            }
        },
        many => {
            let choices = many
                .iter()
                .map(|(candidate, threads)| {
                    let ids = threads
                        .iter()
                        .map(|thread| thread.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    format!(
                        "{} threads=[{}] ({})",
                        crate::target::endpoint_label(&candidate.endpoint),
                        ids,
                        candidate.source
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "multiple auto endpoints have loaded threads for cwd {cwd}; pass --endpoint explicitly: {choices}"
            )
        }
    }
}

pub(crate) async fn resolve_endpoint_for_loaded_thread(
    endpoint: crate::target::Endpoint,
    thread: &str,
) -> anyhow::Result<crate::target::Endpoint> {
    if endpoint != crate::target::Endpoint::Auto {
        let loaded = endpoint_has_loaded_thread(endpoint.clone(), thread).await?;
        if loaded {
            return Ok(endpoint);
        }
        anyhow::bail!(
            "endpoint {} does not have loaded thread {thread}; open the target thread in that endpoint first",
            crate::target::endpoint_label(&endpoint)
        );
    }

    let candidates = crate::target::discover_auto_endpoint_candidates();
    let mut matches = Vec::new();
    let mut failures = Vec::new();
    for candidate in candidates {
        match probe_endpoint_has_loaded_thread(
            candidate.endpoint.clone(),
            thread,
            AUTO_PROBE_TIMEOUT,
        )
        .await
        {
            Ok(true) => matches.push(candidate),
            Ok(false) => {}
            Err(error) => failures.push(format!(
                "{}: {error:#}",
                crate::target::endpoint_label(&candidate.endpoint)
            )),
        }
    }

    match matches.as_slice() {
        [candidate] => Ok(candidate.endpoint.clone()),
        [] => {
            let detail = if failures.is_empty() {
                "no candidate reported the thread as loaded".to_string()
            } else {
                format!("probe failures: {}", failures.join("; "))
            };
            anyhow::bail!(
                "no auto endpoint has loaded thread {thread}; pass --endpoint or open the target thread first ({detail})"
            )
        }
        many => {
            let choices = many
                .iter()
                .map(|candidate| {
                    format!(
                        "{} ({})",
                        crate::target::endpoint_label(&candidate.endpoint),
                        candidate.source
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "multiple auto endpoints have loaded thread {thread}; pass --endpoint explicitly: {choices}"
            )
        }
    }
}

async fn probe_loaded_thread_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
    timeout: Duration,
) -> anyhow::Result<Option<String>> {
    let label = crate::target::endpoint_label(&endpoint);
    auto_probe_timeout(
        format!("loaded-thread probe for {label} cwd {cwd}"),
        timeout,
        loaded_thread_for_cwd(endpoint, cwd),
    )
    .await
}

async fn loaded_thread_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
) -> anyhow::Result<Option<String>> {
    let matches = loaded_threads_for_cwd(endpoint, cwd).await?;
    match matches.as_slice() {
        [thread] => Ok(Some(thread.id.clone())),
        [] => Ok(None),
        many => {
            let ids = many
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("multiple loaded threads match cwd {cwd}: {ids}")
        }
    }
}

async fn probe_loaded_threads_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
    timeout: Duration,
) -> anyhow::Result<Vec<crate::target::ThreadSummary>> {
    let label = crate::target::endpoint_label(&endpoint);
    auto_probe_timeout(
        format!("loaded-threads probe for {label} cwd {cwd}"),
        timeout,
        loaded_threads_for_cwd(endpoint, cwd),
    )
    .await
}

async fn loaded_threads_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
) -> anyhow::Result<Vec<crate::target::ThreadSummary>> {
    let transport = open_endpoint_transport(endpoint).await?;
    let mut client = AppServerClient::new(transport);
    let operation = async {
        client.initialize().await?;
        let loaded_threads = client.thread_loaded_list().await?;
        let result = client.thread_list_by_cwd(cwd).await?;
        let threads = crate::target::parse_thread_list(&result)?;
        let loaded = loaded_threads
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::Ok(
            threads
                .into_iter()
                .filter(|thread| loaded.contains(&thread.id))
                .collect::<Vec<_>>(),
        )
    }
    .await;
    let close_result = client.close().await;
    let threads = operation?;
    close_result?;
    Ok(threads)
}

async fn threads_for_cwd(
    endpoint: crate::target::Endpoint,
    cwd: &str,
) -> anyhow::Result<Vec<crate::target::ThreadSummary>> {
    let transport = open_endpoint_transport(endpoint).await?;
    let mut client = AppServerClient::new(transport);
    let operation = async {
        client.initialize().await?;
        let result = client.thread_list_by_cwd(cwd).await?;
        crate::target::parse_thread_list(&result)
    }
    .await;
    let close_result = client.close().await;
    let threads = operation?;
    close_result?;
    Ok(threads)
}

async fn probe_endpoint_has_loaded_thread(
    endpoint: crate::target::Endpoint,
    thread: &str,
    timeout: Duration,
) -> anyhow::Result<bool> {
    let label = crate::target::endpoint_label(&endpoint);
    auto_probe_timeout(
        format!("loaded-thread-id probe for {label} thread {thread}"),
        timeout,
        endpoint_has_loaded_thread(endpoint, thread),
    )
    .await
}

async fn endpoint_has_loaded_thread(
    endpoint: crate::target::Endpoint,
    thread: &str,
) -> anyhow::Result<bool> {
    let transport = open_endpoint_transport(endpoint).await?;
    let mut client = AppServerClient::new(transport);
    let operation = async {
        client.initialize().await?;
        let loaded_threads = client.thread_loaded_list().await?;
        anyhow::Ok(
            loaded_threads
                .iter()
                .any(|loaded_thread| loaded_thread == thread),
        )
    }
    .await;
    let close_result = client.close().await;
    let loaded = operation?;
    close_result?;
    Ok(loaded)
}

async fn auto_probe_timeout<T, Fut>(
    description: String,
    timeout: Duration,
    future: Fut,
) -> anyhow::Result<T>
where
    Fut: Future<Output = anyhow::Result<T>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("{description} timed out after {}ms", timeout.as_millis()),
    }
}

async fn run_remote_command(
    endpoint: crate::target::Endpoint,
    command: RemoteCommand,
) -> anyhow::Result<i32> {
    let transport = open_endpoint_transport(endpoint).await?;
    let mut client = AppServerClient::new(transport);
    let operation = async {
        client.initialize().await?;
        match command {
            RemoteCommand::Status => {
                let status = client.remote_control_status_read().await?;
                print_remote_status(&status);
            }
            RemoteCommand::Enable => {
                let status = client.remote_control_enable().await?;
                print_remote_status(&status);
            }
            RemoteCommand::Disable => {
                let status = client.remote_control_disable().await?;
                print_remote_status(&status);
            }
            RemoteCommand::PairStart { manual_code } => {
                let pairing = client.remote_control_pairing_start(manual_code).await?;
                print_remote_pairing_start(&pairing);
            }
            RemoteCommand::PairStatus {
                pairing_code,
                manual_pairing_code,
            } => {
                let status = client
                    .remote_control_pairing_status(
                        pairing_code.as_deref(),
                        manual_pairing_code.as_deref(),
                    )
                    .await?;
                print_remote_pairing_status(&status);
            }
            RemoteCommand::Clients { environment_id } => {
                let environment_id = match environment_id {
                    Some(environment_id) => environment_id,
                    None => client
                        .remote_control_status_read()
                        .await?
                        .environment_id
                        .ok_or_else(|| {
                            anyhow!(
                                "remote control has no environmentId; enable remote control or pass --environment-id"
                            )
                        })?,
                };
                let clients = client.remote_control_clients_list(&environment_id).await?;
                print_remote_clients(&clients);
            }
            RemoteCommand::Monitor { count, interval_ms } => {
                let mut emitted = 0;
                loop {
                    let status = client.remote_control_status_read().await?;
                    print_remote_monitor_status(&status);
                    if let Some(environment_id) = status.environment_id.as_deref() {
                        let clients = client.remote_control_clients_list(environment_id).await?;
                        print_remote_monitor_clients(&clients);
                    }
                    emitted += 1;
                    if count.is_some_and(|count| emitted >= count) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                }
            }
            RemoteCommand::Doctor {
                environment_id,
                client_id,
                api_base_url,
                auth_file,
                global_state_file,
                device_key_module,
                skip_refresh,
                skip_backend,
                skip_device_key,
            } => {
                run_remote_doctor(
                    &mut client,
                    RemoteDoctorOptions {
                        environment_id,
                        client_id,
                        api_base_url,
                        auth_file,
                        global_state_file,
                        device_key_module,
                        skip_refresh,
                        skip_backend,
                        skip_device_key,
                    },
                )
                .await?;
            }
            RemoteCommand::Claim {
                manual_pairing_code,
                client_id,
                api_base_url,
                auth_file,
                global_state_file,
                skip_refresh,
            } => {
                if !skip_refresh {
                    client.account_read(true).await?;
                }
                let auth_file =
                    auth_file.unwrap_or_else(crate::remote_control::default_auth_file_path);
                let access_token = crate::remote_control::read_chatgpt_access_token(&auth_file)?;
                let identity = crate::remote_control::parse_chatgpt_auth_identity(&access_token);
                let client_id = match client_id {
                    Some(client_id) => client_id,
                    None => {
                        let global_state_file = global_state_file
                            .unwrap_or_else(crate::remote_control::default_global_state_file_path);
                        crate::remote_control::resolve_enrolled_client_id_from_file_for_identity(
                            &global_state_file,
                            &identity,
                        )?
                    }
                };
                let api_base_url =
                    api_base_url.unwrap_or_else(crate::remote_control::default_api_base_url);
                let claim = crate::remote_control::RemoteControlClientPairClaim {
                    api_base_url,
                    access_token,
                    account_id: identity.account_id,
                    client_id,
                    manual_pairing_code,
                    user_agent: crate::remote_control::default_user_agent(),
                };
                let result = crate::remote_control::claim_remote_control_client_pairing(&claim)
                    .await?;
                print_remote_claim(&claim.client_id, &result);
            }
            RemoteCommand::Connect {
                client_id,
                api_base_url,
                websocket_url,
                auth_file,
                global_state_file,
                device_key_module,
                skip_refresh,
                timeout_ms,
                max_messages,
            } => {
                if !skip_refresh {
                    client.account_read(true).await?;
                }
                let auth_file =
                    auth_file.unwrap_or_else(crate::remote_control::default_auth_file_path);
                let access_token = crate::remote_control::read_chatgpt_access_token(&auth_file)?;
                let identity = crate::remote_control::parse_chatgpt_auth_identity(&access_token);
                let global_state_file =
                    global_state_file.unwrap_or_else(crate::remote_control::default_global_state_file_path);
                let enrollment =
                    crate::remote_control::resolve_enrolled_client_record_from_file_for_identity_and_client_id(
                        &global_state_file,
                        &identity,
                        client_id.as_deref(),
                    )?;
                let api_base_url =
                    api_base_url.unwrap_or_else(crate::remote_control::default_api_base_url);
                let device_key_module =
                    device_key_module.unwrap_or_else(crate::remote_control::default_device_key_module_path);
                let result = crate::remote_control::connect_remote_control_client(
                    &crate::remote_control::RemoteControlClientConnectOptions {
                        api_base_url,
                        websocket_url,
                        access_token,
                        account_id: identity.account_id,
                        enrollment,
                        device_key_module_path: device_key_module,
                        user_agent: crate::remote_control::default_user_agent(),
                        timeout: std::time::Duration::from_millis(timeout_ms),
                        max_messages,
                    },
                )
                .await
                .with_context(|| {
                    "remote connect could not use the local controller enrollment; run `cdxm --target app remote doctor` and re-authorize remote control in Codex App settings if device-key is unavailable"
                })?;
                print_remote_connect(&result);
            }
        }
        anyhow::Ok(())
    }
    .await;
    let close_result = client.close().await;
    operation?;
    close_result?;
    Ok(0)
}

async fn run_remote_doctor(
    client: &mut AppServerClient<Box<dyn crate::transport::AppServerTransport>>,
    options: RemoteDoctorOptions,
) -> anyhow::Result<()> {
    let mut environment_id = options.environment_id.clone();
    match client.remote_control_status_read().await {
        Ok(status) => {
            print_remote_doctor_status(&status);
            if environment_id.is_none() {
                environment_id = status.environment_id.clone();
            }
        }
        Err(error) => {
            print_remote_doctor_error("app-server-status", &error);
        }
    }

    match environment_id.as_deref() {
        Some(environment_id) => match client.remote_control_clients_list(environment_id).await {
            Ok(clients) => print_remote_doctor_app_clients(&clients),
            Err(error) => print_remote_doctor_error("app-server-clients", &error),
        },
        None => print_remote_doctor_error_message(
            "app-server-clients",
            "no environment id from status or --environment-id",
        ),
    }

    if options.skip_refresh {
        print_remote_doctor_skipped("auth-refresh", "--skip-refresh");
    } else {
        match client.account_read(true).await {
            Ok(_) => print_remote_doctor_row(&["doctor", "auth-refresh", "ok"]),
            Err(error) => print_remote_doctor_warn("auth-refresh", &error),
        }
    }

    let auth_file = options
        .auth_file
        .unwrap_or_else(crate::remote_control::default_auth_file_path);
    let access_token = match crate::remote_control::read_chatgpt_access_token(&auth_file) {
        Ok(access_token) => {
            print_remote_doctor_row(&[
                "doctor",
                "auth-file",
                "ok",
                &auth_file.display().to_string(),
            ]);
            Some(access_token)
        }
        Err(error) => {
            print_remote_doctor_error("auth-file", &error);
            None
        }
    };
    let identity = access_token
        .as_deref()
        .map(crate::remote_control::parse_chatgpt_auth_identity);

    if options.skip_backend {
        print_remote_doctor_skipped("backend-clients", "--skip-backend");
    } else if let Some(access_token) = access_token.as_deref() {
        let api_base_url = options
            .api_base_url
            .clone()
            .unwrap_or_else(crate::remote_control::default_api_base_url);
        let account_id = identity
            .as_ref()
            .and_then(|identity| identity.account_id.as_deref());
        match crate::remote_control::list_remote_control_backend_clients(
            &api_base_url,
            access_token,
            account_id,
            &crate::remote_control::default_user_agent(),
        )
        .await
        {
            Ok(clients) => print_remote_doctor_backend_clients(&clients),
            Err(error) => print_remote_doctor_error("backend-clients", &error),
        }
    } else {
        print_remote_doctor_error_message("backend-clients", "auth token unavailable");
    }

    let Some(identity) = identity else {
        print_remote_doctor_error_message("local-enrollment", "auth identity unavailable");
        if !options.skip_device_key {
            print_remote_doctor_error_message("device-key", "local enrollment unavailable");
        }
        return Ok(());
    };

    let global_state_file = options
        .global_state_file
        .unwrap_or_else(crate::remote_control::default_global_state_file_path);
    let enrollment =
        match crate::remote_control::resolve_enrolled_client_record_from_file_for_identity_and_client_id(
            &global_state_file,
            &identity,
            options.client_id.as_deref(),
        ) {
            Ok(enrollment) => {
                print_remote_doctor_row(&[
                    "doctor",
                    "local-enrollment",
                    "ok",
                    &enrollment.client_id,
                    &enrollment.key_id,
                ]);
                Some(enrollment)
            }
            Err(error) => {
                print_remote_doctor_error("local-enrollment", &error);
                None
            }
        };

    if options.skip_device_key {
        print_remote_doctor_skipped("device-key", "--skip-device-key");
    } else if let Some(enrollment) = enrollment.as_ref() {
        let device_key_module = options
            .device_key_module
            .unwrap_or_else(crate::remote_control::default_device_key_module_path);
        let check =
            crate::remote_control::check_device_key_record(&device_key_module, enrollment).await;
        let severity =
            if check.status == crate::remote_control::RemoteControlDeviceKeyStatus::Available {
                "ok"
            } else {
                "warn"
            };
        print_remote_doctor_row(&[
            "doctor",
            "device-key",
            severity,
            &check.client_id,
            &check.key_id,
            check.status.as_str(),
            check.detail.as_deref().unwrap_or("-"),
        ]);
        match check.status {
            crate::remote_control::RemoteControlDeviceKeyStatus::Available => {}
            crate::remote_control::RemoteControlDeviceKeyStatus::Unsupported => {
                print_remote_doctor_row(&[
                    "doctor",
                    "device-key-next",
                    "unsupported-platform",
                    "remote connect cannot act as a phone-like controller on this platform",
                ]);
            }
            crate::remote_control::RemoteControlDeviceKeyStatus::Unavailable
            | crate::remote_control::RemoteControlDeviceKeyStatus::Mismatch => {
                print_remote_doctor_row(&[
                    "doctor",
                    "device-key-next",
                    "repair-local-controller-enrollment",
                    "requires Codex App Settings remote-control re-authorization before cdxm can act like the phone",
                ]);
            }
        }
    } else {
        print_remote_doctor_error_message("device-key", "local enrollment unavailable");
    }

    Ok(())
}

fn print_remote_status(status: &RemoteControlStatus) {
    println!(
        "{}\t{}\t{}\t{}",
        status.status,
        status.server_name,
        status.installation_id,
        status.environment_id.as_deref().unwrap_or("-")
    );
}

fn print_remote_pairing_start(pairing: &RemoteControlPairingStart) {
    println!(
        "{}\t{}\t{}\t{}",
        pairing.environment_id,
        pairing.pairing_code,
        pairing.manual_pairing_code.as_deref().unwrap_or("-"),
        pairing.expires_at
    );
}

fn print_remote_pairing_status(status: &RemoteControlPairingStatus) {
    println!("{}", status.claimed);
}

fn print_remote_clients(clients: &[RemoteControlClient]) {
    for client in clients {
        println!(
            "{}\t{}\t{}\t{}",
            client.client_id,
            client.display_name.as_deref().unwrap_or("-"),
            client.platform.as_deref().unwrap_or("-"),
            client
                .last_seen_at
                .map(|last_seen_at| last_seen_at.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }
}

fn print_remote_monitor_status(status: &RemoteControlStatus) {
    println!(
        "status\t{}\t{}\t{}\t{}",
        status.status,
        status.server_name,
        status.installation_id,
        status.environment_id.as_deref().unwrap_or("-")
    );
}

fn print_remote_monitor_clients(clients: &[RemoteControlClient]) {
    for client in clients {
        println!(
            "client\t{}\t{}\t{}\t{}",
            client.client_id,
            client.display_name.as_deref().unwrap_or("-"),
            client.platform.as_deref().unwrap_or("-"),
            client
                .last_seen_at
                .map(|last_seen_at| last_seen_at.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }
}

fn print_remote_doctor_status(status: &RemoteControlStatus) {
    print_remote_doctor_row(&[
        "doctor",
        "app-server-status",
        "ok",
        &status.status,
        &status.server_name,
        &status.installation_id,
        status.environment_id.as_deref().unwrap_or("-"),
    ]);
}

fn print_remote_doctor_app_clients(clients: &[RemoteControlClient]) {
    let count = clients.len().to_string();
    print_remote_doctor_row(&["doctor", "app-server-clients", "ok", &count]);
    for client in clients {
        let last_seen_at = client
            .last_seen_at
            .map(|last_seen_at| last_seen_at.to_string())
            .unwrap_or_else(|| "-".to_string());
        print_remote_doctor_row(&[
            "doctor",
            "app-server-client",
            &client.client_id,
            client.display_name.as_deref().unwrap_or("-"),
            client.platform.as_deref().unwrap_or("-"),
            &last_seen_at,
        ]);
    }
}

fn print_remote_doctor_backend_clients(clients: &[RemoteControlBackendClient]) {
    let count = clients.len().to_string();
    print_remote_doctor_row(&["doctor", "backend-clients", "ok", &count]);
    for client in clients {
        print_remote_doctor_row(&[
            "doctor",
            "backend-client",
            &client.client_id,
            client.display_name.as_deref().unwrap_or("-"),
            client.platform.as_deref().unwrap_or("-"),
            client.device_type.as_deref().unwrap_or("-"),
            client.last_seen_at.as_deref().unwrap_or("-"),
        ]);
    }
}

fn print_remote_doctor_error(surface: &str, error: &anyhow::Error) {
    print_remote_doctor_error_message(surface, &error.to_string());
}

fn print_remote_doctor_warn(surface: &str, error: &anyhow::Error) {
    print_remote_doctor_row(&["doctor", surface, "warn", &error.to_string()]);
}

fn print_remote_doctor_error_message(surface: &str, message: &str) {
    print_remote_doctor_row(&["doctor", surface, "error", message]);
}

fn print_remote_doctor_skipped(surface: &str, reason: &str) {
    print_remote_doctor_row(&["doctor", surface, "skipped", reason]);
}

fn print_remote_doctor_row(fields: &[&str]) {
    let sanitized = fields
        .iter()
        .map(|field| sanitize_remote_doctor_field(field))
        .collect::<Vec<_>>();
    println!("{}", sanitized.join("\t"));
}

fn sanitize_remote_doctor_field(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

fn print_remote_claim(client_id: &str, _result: &serde_json::Value) {
    println!("claimed\t{client_id}");
}

fn print_remote_connect(result: &crate::remote_control::RemoteControlClientConnectResult) {
    println!(
        "connected\t{}\t{}\t{}",
        result.client_id,
        result.token_expires_at,
        result.scopes.join(",")
    );
    println!("device-key-proof\t{}", result.proof_algorithm);
    for message in &result.messages {
        println!("message\t{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};

    async fn start_hanging_server() -> crate::target::Endpoint {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut ws) = accept_async(stream).await else {
                return;
            };
            if ws.next().await.is_some() {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        crate::target::Endpoint::Explicit(format!("ws://{addr}"))
    }

    async fn start_loaded_thread_server(cwd: &'static str) -> crate::target::Endpoint {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut ws) = accept_async(stream).await else {
                return;
            };
            while let Some(message) = ws.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        if ws
                            .send(Message::Text(
                                json!({ "id": request["id"], "result": {} })
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    "initialized" => {}
                    "thread/loaded/list" => {
                        if ws
                            .send(Message::Text(
                                json!({
                                    "id": request["id"],
                                    "result": { "data": ["thread-good"] }
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    "thread/list" => {
                        if ws
                            .send(Message::Text(
                                json!({
                                    "id": request["id"],
                                    "result": {
                                        "data": [
                                            {
                                                "id": "thread-good",
                                                "title": "Good",
                                                "cwd": cwd
                                            }
                                        ]
                                    }
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        });
        crate::target::Endpoint::Explicit(format!("ws://{addr}"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auto_cwd_thread_resolution_skips_unresponsive_endpoint() {
        let hanging = start_hanging_server().await;
        let good = start_loaded_thread_server("/tmp/project").await;
        let candidates = vec![
            crate::target::EndpointCandidate {
                endpoint: hanging,
                source: "hanging-test".to_string(),
            },
            crate::target::EndpointCandidate {
                endpoint: good,
                source: "good-test".to_string(),
            },
        ];

        let threads = resolve_threads_for_cwd_from_candidates(
            candidates,
            "/tmp/project",
            Duration::from_millis(250),
        )
        .await
        .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "thread-good");
    }

    #[test]
    fn auto_probe_timeout_allows_real_app_thread_listing_latency() {
        assert!(
            AUTO_PROBE_TIMEOUT >= Duration::from_secs(5),
            "cwd auto resolution calls thread/loaded/list and thread/list; Windows Codex app-server can exceed short 1500ms probes"
        );
    }

    #[test]
    fn parses_agmsg_consumer_processes_from_ps_text() {
        let text = r#"
  100 /Users/ysk411/.cargo/bin/cdxm agmsg watch --team emeria --name steve --cwd /Users/ysk411/dev/emeriasaga
  101 /Users/ysk411/.cargo/bin/cdxm agmsg watch --team emeria --name advisor --cwd /Users/ysk411/dev/emeriasaga
  102 /Users/ysk411/.cargo/bin/cdxm agmsg watch --team emeria --name reviewer --cwd /Users/ysk411/dev/emeriasaga
  103 /Users/ysk411/.cargo/bin/codex-monitor agmsg watch --team emeria --name monitor --cwd /Users/ysk411/dev/emeriasaga
  104 /Users/ysk411/.agents/skills/agmsg/scripts/watch.sh /Users/ysk411/dev/emeriasaga codex game-maker
  105 /bin/zsh
"#;

        let consumers = parse_agmsg_consumer_processes(text);

        assert_eq!(consumers.len(), 5);
        assert_eq!(consumers[0].pid, 100);
        assert_eq!(consumers[0].kind, "codex-monitor-agmsg-watch");
        assert_eq!(consumers[0].team.as_deref(), Some("emeria"));
        assert_eq!(consumers[0].name.as_deref(), Some("steve"));
        assert_eq!(consumers[0].thread.as_deref(), None);
        assert_eq!(consumers[1].name.as_deref(), Some("advisor"));
        assert_eq!(consumers[2].name.as_deref(), Some("reviewer"));
        assert_eq!(consumers[3].name.as_deref(), Some("monitor"));
        assert_eq!(consumers[4].kind, "agmsg-watch-sh");
        assert_eq!(consumers[4].name.as_deref(), Some("game-maker"));
    }

    #[test]
    fn parses_agmsg_consumer_thread_from_ps_text() {
        let text = r#"
  52232 /Users/ysk411/.cargo/bin/cdxm agmsg watch --team emeria --name codex --thread 019ede87-2268-7951-a2ec-9b59b0074037 --cwd /Users/ysk411/dev/emeriasaga
"#;

        let consumers = parse_agmsg_consumer_processes(text);

        assert_eq!(consumers.len(), 1);
        assert_eq!(consumers[0].team.as_deref(), Some("emeria"));
        assert_eq!(consumers[0].name.as_deref(), Some("codex"));
        assert_eq!(
            consumers[0].thread.as_deref(),
            Some("019ede87-2268-7951-a2ec-9b59b0074037")
        );
    }

    #[test]
    fn parses_windows_monitor_watch_agmsg_consumer() {
        let text = r#"
  56536 "C:\Users\ytvar\.codex-monitor\bin\cdxm.exe" monitor watch agmsg --team cdxm --name codex1 --cwd C:/Users/ytvar/dev/codex-monitor --mode auto --thread 019ef1b8-4d96-70a1-babc-0bd63ef5bc41
"#;

        let consumers = parse_agmsg_consumer_processes(text);

        assert_eq!(consumers.len(), 1);
        assert_eq!(consumers[0].pid, 56536);
        assert_eq!(consumers[0].kind, "codex-monitor-agmsg-watch");
        assert_eq!(consumers[0].team.as_deref(), Some("cdxm"));
        assert_eq!(consumers[0].name.as_deref(), Some("codex1"));
        assert_eq!(
            consumers[0].thread.as_deref(),
            Some("019ef1b8-4d96-70a1-babc-0bd63ef5bc41")
        );
    }
}
