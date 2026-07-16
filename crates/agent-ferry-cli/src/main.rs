use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent_ferry_core::{
    AgentFerryPaths, NativeHostManifest, read_native_host_manifest, send_ipc_request,
};
use agent_ferry_hermes::{ConnectionDiagnosis, DiagnosisState, load_connections};
use agent_ferry_protocol::{
    Command, ConnectorKind, HandoffTargetState, HandoffTargetStatus, HostRequest, PROTOCOL_VERSION,
    ResponseOutcome, ServiceState, StatusResult,
};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "aferry", version, about = "Agent Ferry 本机配置与诊断")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// 只读检查当前安装状态并给出下一步命令
    Setup(OutputArgs),
    /// 只读执行完整健康检查；发现问题时返回非零退出码
    Doctor(OutputArgs),
    /// 管理 Chrome Native Messaging Host 注册
    NativeHost {
        #[command(subcommand)]
        command: NativeHostCommand,
    },
    /// 管理 Remote Hermes Connection
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
}

#[derive(Debug, Clone, Args)]
struct OutputArgs {
    /// 输出稳定 JSON，供安装器或后续 GUI 使用
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum NativeHostCommand {
    /// 显式注册 Native Host；setup/doctor 本身不会修改系统
    Register {
        #[arg(long)]
        extension_id: String,
        #[arg(long)]
        host_path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectionCommand {
    /// 新增一种远程 Connection
    Add {
        #[command(subcommand)]
        kind: ConnectionKind,
    },
    /// 列出不含凭据值的 Connection 配置
    List(OutputArgs),
    /// 通过 capability discovery 诊断一个或全部 Connection
    Doctor {
        /// Connection ID 或名称；省略时诊断全部
        identifier: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 删除 Connection 及其 Keychain 凭据
    Remove {
        /// Connection ID 或名称
        identifier: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectionKind {
    /// 通过 Direct URL 连接用户已有的 Hermes API Server
    Hermes {
        #[arg(long)]
        name: String,
        /// Hermes API Server 根 URL，可包含反向代理路径前缀，但不要包含 /v1
        #[arg(long)]
        url: String,
        #[arg(long)]
        model: Option<String>,
        /// 从 stdin 读取 Bearer Token，避免进入 shell history 和进程列表
        #[arg(long)]
        token_stdin: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
struct SetupReport {
    core: Check,
    daemon: Check,
    native_host: Check,
    chrome_extension: Check,
    hermes_connections: Vec<ConnectionDiagnosis>,
    next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Check {
    state: CheckState,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct ConnectionListItem {
    id: String,
    name: String,
    kind: &'static str,
    transport: &'static str,
    endpoint: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckState {
    Ready,
    NotDetected,
    Broken,
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run(cli) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("aferry: {error}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32, CliError> {
    match cli.command {
        None => {
            println!("Agent Ferry {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some(CliCommand::Setup(args)) => {
            let report = collect_report()?;
            print_report(&report, args.json)?;
            Ok(0)
        }
        Some(CliCommand::Doctor(args)) => {
            let report = collect_report()?;
            let healthy = report.daemon.state == CheckState::Ready
                && report.native_host.state == CheckState::Ready
                && report.chrome_extension.state == CheckState::Ready;
            print_report(&report, args.json)?;
            Ok(i32::from(!healthy))
        }
        Some(CliCommand::NativeHost {
            command:
                NativeHostCommand::Register {
                    extension_id,
                    host_path,
                },
        }) => {
            register_native_host(&extension_id, &host_path)?;
            Ok(0)
        }
        Some(CliCommand::Connection { command }) => run_connection_command(command),
    }
}

fn collect_report() -> Result<SetupReport, CliError> {
    let paths = AgentFerryPaths::discover()?;
    let native_host = inspect_native_host(&paths);
    let daemon_response = send_daemon_command(&paths, Command::Status).ok();

    let (daemon, chrome_extension, target_statuses) = match daemon_response {
        Some(result) => (
            Check {
                state: CheckState::Ready,
                detail: format!("agentferryd {} 已连接", result.core_version),
            },
            service_check(&result.chrome_extension, "Chrome 扩展已连接过 Native Host"),
            result.targets,
        ),
        None => (
            Check {
                state: CheckState::NotDetected,
                detail: "无法连接 agentferryd".to_owned(),
            },
            Check {
                state: CheckState::NotDetected,
                detail: "daemon 不可用，无法确认扩展连接".to_owned(),
            },
            Vec::new(),
        ),
    };

    let mut next_actions = Vec::new();
    if daemon.state != CheckState::Ready {
        next_actions.push("启动 agentferryd，然后重新运行 aferry doctor".to_owned());
    }
    if native_host.state != CheckState::Ready {
        next_actions.push(
            "运行 aferry native-host register --extension-id <id> --host-path <absolute-path>"
                .to_owned(),
        );
    }
    if chrome_extension.state != CheckState::Ready {
        next_actions.push("打开 Agent Ferry Chrome 扩展以完成连接检查".to_owned());
    }
    let hermes_connections = diagnoses_from_targets(&paths, None, &target_statuses)?;
    if hermes_connections.is_empty() {
        next_actions.push(
            "运行 aferry connection add hermes --name <name> --url <url> --token-stdin".to_owned(),
        );
    } else if hermes_connections
        .iter()
        .any(|diagnosis| diagnosis.state != DiagnosisState::Ready)
    {
        next_actions.push("运行 aferry connection doctor 查看 Hermes 修复建议".to_owned());
    }

    Ok(SetupReport {
        core: Check {
            state: CheckState::Ready,
            detail: format!("aferry {}", env!("CARGO_PKG_VERSION")),
        },
        daemon,
        native_host,
        chrome_extension,
        hermes_connections,
        next_actions,
    })
}

fn run_connection_command(command: ConnectionCommand) -> Result<i32, CliError> {
    let paths = AgentFerryPaths::discover()?;
    match command {
        ConnectionCommand::Add {
            kind:
                ConnectionKind::Hermes {
                    name,
                    url,
                    model,
                    token_stdin,
                },
        } => {
            if !token_stdin {
                return Err(CliError::TokenStdinRequired);
            }
            let token = read_token_from_stdin()?;
            let token = String::from_utf8(token).map_err(|_| CliError::TokenNotUtf8)?;
            let result = send_daemon_command(
                &paths,
                Command::ConnectionAdd {
                    name: name.clone(),
                    base_url: url,
                    model,
                    token,
                },
            )?;
            let id = result
                .targets
                .iter()
                .find(|target| target.name == name)
                .map_or("unknown", |target| target.id.as_str());
            println!("已添加 Hermes Connection: {name} ({id})");
            Ok(0)
        }
        ConnectionCommand::List(output) => {
            let connections = load_connections(&paths.hermes_connections)?;
            let mut items = Vec::with_capacity(connections.connections.len());
            for connection in connections.connections {
                items.push(ConnectionListItem {
                    id: connection.id,
                    name: connection.name,
                    kind: "remote_hermes",
                    transport: "direct",
                    endpoint: connection.endpoint.base_url.to_string(),
                    model: connection.endpoint.model,
                });
            }
            if output.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else if items.is_empty() {
                println!("尚未配置 Hermes Connection");
            } else {
                for item in items {
                    println!("{}  {}  {}", item.id, item.name, item.endpoint);
                }
            }
            Ok(0)
        }
        ConnectionCommand::Doctor { identifier, output } => {
            let result = send_daemon_command(&paths, Command::Status)?;
            let diagnoses = diagnoses_from_targets(&paths, identifier.as_deref(), &result.targets)?;
            if output.json {
                println!("{}", serde_json::to_string_pretty(&diagnoses)?);
            } else if diagnoses.is_empty() {
                println!("尚未配置 Hermes Connection");
            } else {
                for diagnosis in &diagnoses {
                    println!(
                        "{}  {}  {}  {}",
                        diagnosis.id,
                        diagnosis.name,
                        diagnosis_state_symbol(diagnosis.state),
                        diagnosis.detail
                    );
                    if !diagnosis.capabilities.is_empty() {
                        println!("  capabilities: {}", diagnosis.capabilities.join(", "));
                    }
                }
            }
            Ok(i32::from(
                diagnoses.is_empty()
                    || diagnoses
                        .iter()
                        .any(|diagnosis| diagnosis.state != DiagnosisState::Ready),
            ))
        }
        ConnectionCommand::Remove { identifier } => {
            send_daemon_command(
                &paths,
                Command::ConnectionRemove {
                    identifier: identifier.clone(),
                },
            )?;
            println!("已删除 Hermes Connection: {identifier}");
            Ok(0)
        }
    }
}

fn read_token_from_stdin() -> Result<Vec<u8>, CliError> {
    const MAX_TOKEN_BYTES: u64 = 16 * 1024;
    let mut token = Vec::new();
    io::stdin()
        .take(MAX_TOKEN_BYTES + 1)
        .read_to_end(&mut token)?;
    if token.len() as u64 > MAX_TOKEN_BYTES {
        return Err(CliError::TokenTooLarge);
    }
    while matches!(token.last(), Some(b'\n' | b'\r')) {
        token.pop();
    }
    if token.is_empty() {
        return Err(CliError::EmptyToken);
    }
    Ok(token)
}

fn diagnoses_from_targets(
    paths: &AgentFerryPaths,
    identifier: Option<&str>,
    targets: &[HandoffTargetStatus],
) -> Result<Vec<ConnectionDiagnosis>, CliError> {
    let connections = load_connections(&paths.hermes_connections)?;
    let selected = connections
        .connections
        .into_iter()
        .filter(|connection| {
            identifier.is_none_or(|value| connection.id == value || connection.name == value)
        })
        .collect::<Vec<_>>();
    if let Some(identifier) = identifier
        && selected.is_empty()
    {
        return Err(CliError::ConnectionNotFound(identifier.to_owned()));
    }
    let mut diagnoses = Vec::with_capacity(selected.len());
    for connection in selected {
        let target = targets.iter().find(|target| target.id == connection.id);
        diagnoses.push(match target {
            Some(target) => ConnectionDiagnosis {
                id: target.id.clone(),
                name: target.name.clone(),
                state: target_state_to_diagnosis(target.state),
                detail: target_state_detail(target.state).to_owned(),
                capabilities: target.capabilities.clone(),
            },
            None => ConnectionDiagnosis {
                id: connection.id,
                name: connection.name,
                state: DiagnosisState::ConnectionFailed,
                detail: "daemon 未返回该 Connection 的诊断状态".to_owned(),
                capabilities: Vec::new(),
            },
        });
    }
    Ok(diagnoses)
}

fn send_daemon_command(
    paths: &AgentFerryPaths,
    command: Command,
) -> Result<StatusResult, CliError> {
    let request = HostRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        command,
    };
    let response = send_ipc_request(paths, ConnectorKind::Cli, serde_json::to_value(request)?)?;
    match response.outcome {
        ResponseOutcome::Success { result } => Ok(result),
        ResponseOutcome::Failure { error } => Err(CliError::DaemonRejected(error.message)),
    }
}

const fn target_state_to_diagnosis(state: HandoffTargetState) -> DiagnosisState {
    match state {
        HandoffTargetState::Ready => DiagnosisState::Ready,
        HandoffTargetState::CredentialMissing => DiagnosisState::CredentialMissing,
        HandoffTargetState::AuthenticationFailed => DiagnosisState::AuthenticationFailed,
        HandoffTargetState::ConnectionFailed => DiagnosisState::ConnectionFailed,
        HandoffTargetState::Incompatible => DiagnosisState::Incompatible,
    }
}

const fn target_state_detail(state: HandoffTargetState) -> &'static str {
    match state {
        HandoffTargetState::Ready => "Hermes capability discovery 通过",
        HandoffTargetState::CredentialMissing => "Keychain 中未找到 Hermes Bearer Token",
        HandoffTargetState::AuthenticationFailed => "Hermes 拒绝 Bearer Token",
        HandoffTargetState::ConnectionFailed => "无法连接 Hermes capability endpoint",
        HandoffTargetState::Incompatible => "服务器缺少 run_submission 或 run_status",
    }
}

const fn diagnosis_state_symbol(state: DiagnosisState) -> &'static str {
    match state {
        DiagnosisState::Ready => "ready",
        DiagnosisState::CredentialMissing => "credential_missing",
        DiagnosisState::AuthenticationFailed => "authentication_failed",
        DiagnosisState::ConnectionFailed => "connection_failed",
        DiagnosisState::Incompatible => "incompatible",
    }
}

fn inspect_native_host(paths: &AgentFerryPaths) -> Check {
    let Ok(manifest) = read_native_host_manifest(&paths.native_host_manifest) else {
        return Check {
            state: CheckState::NotDetected,
            detail: format!("未找到 {}", paths.native_host_manifest.display()),
        };
    };
    if manifest.name != agent_ferry_protocol::NATIVE_HOST_NAME
        || manifest.transport_type != "stdio"
        || manifest.allowed_origins.len() != 1
        || !manifest.allowed_origins[0].starts_with("chrome-extension://")
        || !manifest.allowed_origins[0].ends_with('/')
    {
        return Check {
            state: CheckState::Broken,
            detail: "Native Host manifest 的名称、类型或扩展 allowlist 无效".to_owned(),
        };
    }
    let Ok(metadata) = fs::metadata(&manifest.path) else {
        return Check {
            state: CheckState::Broken,
            detail: format!("Native Host 不存在: {}", manifest.path.display()),
        };
    };
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Check {
            state: CheckState::Broken,
            detail: format!("Native Host 不可执行: {}", manifest.path.display()),
        };
    }
    Check {
        state: CheckState::Ready,
        detail: format!(
            "{} → {}",
            paths.native_host_manifest.display(),
            manifest.path.display()
        ),
    }
}

fn service_check(state: &ServiceState, ready_detail: &str) -> Check {
    match state {
        ServiceState::Ready => Check {
            state: CheckState::Ready,
            detail: ready_detail.to_owned(),
        },
        ServiceState::NotDetected => Check {
            state: CheckState::NotDetected,
            detail: "尚未检测到".to_owned(),
        },
    }
}

fn register_native_host(extension_id: &str, host_path: &Path) -> Result<(), CliError> {
    if extension_id.len() != 32
        || !extension_id
            .bytes()
            .all(|character| (b'a'..=b'p').contains(&character))
    {
        return Err(CliError::InvalidExtensionId);
    }
    let host_path = host_path.canonicalize()?;
    let metadata = fs::metadata(&host_path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(CliError::HostNotExecutable(host_path));
    }
    let paths = AgentFerryPaths::discover()?;
    let manifest = NativeHostManifest::new(host_path, extension_id);
    let parent = paths
        .native_host_manifest
        .parent()
        .ok_or_else(|| io::Error::other("Native Host manifest 没有父目录"))?;
    fs::create_dir_all(parent)?;

    // 先写临时文件再 rename，避免 Chrome 恰好读取到半个 JSON manifest。
    let temporary = paths
        .native_host_manifest
        .with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec_pretty(&manifest)?)?;
    fs::rename(&temporary, &paths.native_host_manifest)?;
    println!(
        "已注册 Native Host: {}",
        paths.native_host_manifest.display()
    );
    Ok(())
}

fn print_report(report: &SetupReport, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!(
        "Agent Ferry Core       {}  {}",
        state_symbol(report.core.state),
        report.core.detail
    );
    println!(
        "agentferryd            {}  {}",
        state_symbol(report.daemon.state),
        report.daemon.detail
    );
    println!(
        "Chrome Native Host     {}  {}",
        state_symbol(report.native_host.state),
        report.native_host.detail
    );
    println!(
        "Chrome Extension       {}  {}",
        state_symbol(report.chrome_extension.state),
        report.chrome_extension.detail
    );
    if report.hermes_connections.is_empty() {
        println!("Remote Hermes         not_configured  尚未配置 Connection");
    } else {
        for diagnosis in &report.hermes_connections {
            println!(
                "Remote Hermes         {}  {}: {}",
                diagnosis_state_symbol(diagnosis.state),
                diagnosis.name,
                diagnosis.detail
            );
        }
    }
    if !report.next_actions.is_empty() {
        println!("\nNext actions");
        for action in &report.next_actions {
            println!("  {action}");
        }
    }
    Ok(())
}

const fn state_symbol(state: CheckState) -> &'static str {
    match state {
        CheckState::Ready => "ready",
        CheckState::NotDetected => "not_detected",
        CheckState::Broken => "broken",
    }
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Hermes(#[from] agent_ferry_hermes::HermesError),
    #[error("Chrome extension id 必须是 32 个 a-p 小写字符")]
    InvalidExtensionId,
    #[error("Native Host 不可执行: {0}")]
    HostNotExecutable(PathBuf),
    #[error("请使用 --token-stdin 从 stdin 提供 Hermes Bearer Token")]
    TokenStdinRequired,
    #[error("Hermes Bearer Token 不能为空")]
    EmptyToken,
    #[error("Hermes Bearer Token 必须是 UTF-8")]
    TokenNotUtf8,
    #[error("Hermes Bearer Token 超过 16 KiB 上限")]
    TokenTooLarge,
    #[error("未找到 Hermes Connection: {0}")]
    ConnectionNotFound(String),
    #[error("agentferryd 拒绝命令: {0}")]
    DaemonRejected(String),
}
