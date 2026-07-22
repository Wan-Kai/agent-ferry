use std::env;
use std::fs;
use std::io::{self, IsTerminal as _, Read, Write as _};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent_ferry_claude::{
    ClaudeDiagnosis, ClaudeDocument, ClaudeState, ClaudeTaskEvent, MAX_CLAUDE_DOCUMENT_BYTES,
};
use agent_ferry_codex::{
    CodexDiagnosis, CodexDocument, CodexState, CodexSurface, CodexTaskEvent,
    MAX_CODEX_DOCUMENT_BYTES,
};
use agent_ferry_core::workspace::{WorkspaceState, diagnose as diagnose_workspace};
use agent_ferry_core::{
    AgentFerryPaths, DataMigrationOutcome, NativeHostManifest, migrate_legacy_data,
    open_ipc_stream, read_native_host_manifest, send_ipc_request,
};
use agent_ferry_hermes::{ConnectionDiagnosis, DiagnosisState, load_connections};
#[cfg(debug_assertions)]
use agent_ferry_hermes::{CredentialStore, DevelopmentCredentialStore, KeychainCredentialStore};
use agent_ferry_opencode::{
    DEFAULT_OPENCODE_MODEL, MAX_OPENCODE_DOCUMENT_BYTES, OpenCodeDiagnosis, OpenCodeDocument,
    OpenCodeState, OpenCodeTaskEvent,
};
use agent_ferry_protocol::{
    Command, ConnectionTransportConfig, ConnectorKind, FrameError, HandoffEvent, HandoffEventKind,
    HandoffTargetState, HandoffTargetStatus, HostRequest, HostResponse, MAX_HERMES_RUN_INPUT_BYTES,
    PROTOCOL_VERSION, ResponseOutcome, ServiceState, StatusResult, read_json_frame,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use uuid::Uuid;

mod hermes_setup;
mod service;
mod uninstall;
mod update;

const PUBLIC_CHROME_EXTENSION_ID: &str = "ommpdijpcidnicpbalkpnggoljhapcel";

#[derive(Debug, Parser)]
#[command(name = "aferry", version, about = "Agent Ferry 本机配置与诊断")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// 激活后台服务并注册正式 Chrome Native Host
    Activate(ActivateArgs),
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
    /// 使用最少参数连接本地或远程 Agent
    Connect {
        #[command(subcommand)]
        command: ConnectCommand,
    },
    /// 管理本地 Agent 绑定
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// 管理本地 Agent 可使用的固定 Workspace
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// 管理 Agent Ferry 自己的用户数据
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
    /// 管理 macOS `agentferryd` `LaunchAgent`
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// 使用当前版本携带的受信任安装器升级 Agent Ferry
    Update {
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        manifest_url: Option<String>,
    },
    /// 卸载 Agent Ferry；默认保留用户数据和凭据
    Uninstall {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 仅 Debug 构建提供的本地开发辅助命令
    #[cfg(debug_assertions)]
    Dev {
        #[command(subcommand)]
        command: DevCommand,
    },
}

#[derive(Debug, Clone, Args)]
struct ActivateArgs {
    /// 输出稳定 JSON，供安装器或后续 GUI 使用；JSON 模式不会进入交互配置
    #[arg(long)]
    json: bool,
    /// 接受检测结果并使用当前目录或 --workspace 作为默认运行位置
    #[arg(long, conflicts_with = "json")]
    yes: bool,
    /// 一键连接本地 Agent 时保存为默认运行位置
    #[arg(long, requires = "yes")]
    workspace: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// 安装并加载当前用户的 `LaunchAgent`
    Install {
        #[arg(long)]
        daemon_path: Option<PathBuf>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 加载已经安装的 `LaunchAgent`
    Start(OutputArgs),
    /// 停止并卸载当前 LaunchAgent，保留 plist
    Stop(OutputArgs),
    /// 重新加载当前 `LaunchAgent`
    Restart(OutputArgs),
    /// 查看 launchd 状态、PID 和日志路径
    Status(OutputArgs),
    /// 输出最近的 daemon 日志
    Logs {
        #[arg(long, default_value_t = 100)]
        lines: usize,
    },
    /// 停止服务并删除 `LaunchAgent` plist，保留日志和用户数据
    Uninstall(OutputArgs),
}

#[derive(Debug, Subcommand)]
enum DataCommand {
    /// 将早期开发版本的数据迁移到 ~/.agent-ferry
    Migrate(OutputArgs),
}

#[cfg(debug_assertions)]
#[derive(Debug, Subcommand)]
enum DevCommand {
    /// 将已配置 Hermes 的凭据复制到私有开发文件
    CacheHermesCredentials,
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    /// 保存一个已经存在的本地目录
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: PathBuf,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 列出全部 Workspace 与当前状态
    List(OutputArgs),
    /// 诊断一个或全部 Workspace
    Doctor {
        identifier: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 只删除配置引用，不删除真实目录
    Remove { identifier: String },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// 管理用户自行安装的 Claude Code；Agent Ferry 不代为安装
    Claude {
        #[command(subcommand)]
        command: ClaudeCommand,
    },
    /// 管理用户自行安装的 OpenCode；Agent Ferry 不代为安装
    Opencode {
        #[command(subcommand)]
        command: OpenCodeCommand,
    },
    /// 管理用户自行安装或桌面 App 内置的 Codex；Agent Ferry 不代为安装
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CodexRunSurface {
    Cli,
    App,
}

impl From<CodexRunSurface> for CodexSurface {
    fn from(value: CodexRunSurface) -> Self {
        match value {
            CodexRunSurface::Cli => Self::Cli,
            CodexRunSurface::App => Self::App,
        }
    }
}

#[derive(Debug, Subcommand)]
enum CodexCommand {
    /// 从 PATH 与官方桌面 App 位置发现候选；单一且兼容时自动绑定
    Detect(OutputArgs),
    /// 使用绝对路径明确绑定 Codex
    Bind {
        #[arg(long)]
        path: PathBuf,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 诊断已绑定路径、登录、exec 与 App Server 能力
    Doctor(OutputArgs),
    /// 在指定 Workspace 启动全新的 Codex CLI 或 App Server 任务
    Run {
        #[arg(long, value_enum)]
        surface: CodexRunSurface,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        document_file: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_url: String,
        /// 从 stdin 读取 Prompt，避免进入 argv 和 shell history
        #[arg(long)]
        prompt_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OpenCodeCommand {
    /// 从 PATH 发现候选；单一且兼容时自动绑定
    Detect {
        #[arg(long, default_value = DEFAULT_OPENCODE_MODEL)]
        model: String,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 使用绝对路径与显式模型明确绑定 `OpenCode`
    Bind {
        #[arg(long)]
        path: PathBuf,
        #[arg(long, default_value = DEFAULT_OPENCODE_MODEL)]
        model: String,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 诊断已绑定路径、run flags 和显式模型
    Doctor(OutputArgs),
    /// 在指定 Workspace 启动一个全新的 unrestricted 一次性任务
    Run {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        document_file: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_url: String,
        /// 从 stdin 读取 Prompt，避免进入 argv 和 shell history
        #[arg(long)]
        prompt_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ClaudeCommand {
    /// 从 PATH 发现候选；单一且兼容时自动绑定
    Detect(OutputArgs),
    /// 使用绝对路径明确绑定 Claude Code
    Bind {
        #[arg(long)]
        path: PathBuf,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// 诊断已绑定路径、Print Mode flags 和认证状态
    Doctor(OutputArgs),
    /// 在指定 Workspace 启动一个全新的 unrestricted Print Task
    Run {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        document_file: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_url: String,
        /// 从 stdin 读取 Prompt，避免进入 argv 和 shell history
        #[arg(long)]
        prompt_stdin: bool,
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
    /// 通过 SSH 准备标准 Docker Hermes，并创建 Connection
    Setup {
        #[command(subcommand)]
        kind: ConnectionSetupKind,
    },
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
    /// 不经过浏览器，直接提交一个 Hermes Run 并观察到终态
    Run {
        /// Connection ID 或名称
        identifier: String,
        /// 从文件读取完整 input
        #[arg(long, conflicts_with = "input_stdin")]
        input_file: Option<PathBuf>,
        /// 从 stdin 读取完整 input
        #[arg(long, conflicts_with = "input_file")]
        input_stdin: bool,
    },
    /// 删除 Connection 及其 Keychain 凭据
    Remove {
        /// Connection ID 或名称
        identifier: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectCommand {
    /// 通过 SSH 自动准备并连接标准 Docker Hermes
    Hermes {
        /// OpenSSH 目标，例如 root@example.com 或 ~/.ssh/config 中的 Host
        ssh_host: String,
        /// 浏览器中显示的连接名称；默认根据服务器地址生成
        #[arg(long)]
        name: Option<String>,
        /// 跳过远端变更计划确认
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectionSetupKind {
    /// 识别并安全配置官方 Docker gateway 容器
    Hermes {
        #[arg(long)]
        name: String,
        /// 可通过公钥非交互登录的 user@host 或 ~/.ssh/config Host
        #[arg(long)]
        ssh_host: String,
        /// 远端 Docker 容器名称
        #[arg(long, default_value = "hermes")]
        container: String,
        /// 跳过交互确认；仅供用户已审阅计划后的自动化执行
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectionKind {
    /// 连接用户已有的 Hermes API Server；默认 Direct，可显式选择 SSH Tunnel
    Hermes {
        #[arg(long)]
        name: String,
        /// Hermes API Server 根 URL，可包含反向代理路径前缀，但不要包含 /v1
        #[arg(long)]
        url: String,
        #[arg(long)]
        model: Option<String>,
        /// 通过该 ~/.ssh/config host 建立 Tunnel；省略时使用 Direct
        #[arg(long)]
        ssh_host: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    ssh_host: Option<String>,
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
        Some(CliCommand::Activate(args)) => run_activate(&args),
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
        Some(CliCommand::Connect { command }) => run_connect_command(command),
        Some(CliCommand::Agent { command }) => run_agent_command(command),
        Some(CliCommand::Workspace { command }) => run_workspace_command(command),
        Some(CliCommand::Data { command }) => run_data_command(command),
        Some(CliCommand::Service { command }) => run_service_command(command),
        Some(CliCommand::Update {
            version,
            manifest_url,
        }) => update::run(version.as_deref(), manifest_url.as_deref()).map_err(Into::into),
        Some(CliCommand::Uninstall { purge, yes, output }) => {
            let report = uninstall::run(purge, yes)?;
            if output.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                if report.program == uninstall::RemovalState::ManagedExternally {
                    println!("Agent Ferry 运行资源已移除；程序文件仍由 Homebrew 管理");
                    println!("  下一步: brew uninstall Wan-Kai/tap/agent-ferry");
                } else {
                    println!("Agent Ferry 已卸载");
                }
                println!("  用户数据: {:?}", report.user_data);
                println!("  日志: {:?}", report.logs);
            }
            Ok(0)
        }
        #[cfg(debug_assertions)]
        Some(CliCommand::Dev { command }) => run_dev_command(&command),
    }
}

fn run_activate(args: &ActivateArgs) -> Result<i32, CliError> {
    let executable_dir = env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("aferry 可执行文件没有父目录"))?
        .to_path_buf();
    let daemon_path = executable_dir.join("agentferryd");
    let host_path = executable_dir.join("agentferry-host");

    // 激活必须发生在用户自己的终端环境中，避免 Homebrew post_install 沙箱把资源写入临时 HOME。
    // 两个步骤都是幂等的；任一步中断后重新执行本命令即可收敛到完整状态。
    register_native_host(PUBLIC_CHROME_EXTENSION_ID, &host_path)?;
    let manager = service::ServiceManager::discover()?;
    let report = manager.install(Some(&daemon_path))?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "state": "activated",
                "extension_id": PUBLIC_CHROME_EXTENSION_ID,
                "service": report,
            })
        );
    } else {
        println!("Agent Ferry 已激活");
        println!("  Chrome 扩展: {PUBLIC_CHROME_EXTENSION_ID}");
        println!("  后台服务: {:?}", report.state);
        offer_local_agent_setup(args)?;
        println!("  下一步: aferry doctor");
    }
    Ok(0)
}

#[derive(Debug, Default)]
struct LocalAgentOffer {
    claude: Option<PathBuf>,
    opencode: Option<PathBuf>,
    codex: Option<PathBuf>,
    rows: Vec<String>,
    notes: Vec<String>,
}

fn offer_local_agent_setup(args: &ActivateArgs) -> Result<(), CliError> {
    let paths = AgentFerryPaths::discover()?;
    let offer = inspect_unbound_local_agents(&paths)?;
    if offer.rows.is_empty() {
        if !offer.notes.is_empty() {
            println!();
            for note in offer.notes {
                println!("  {note}");
            }
        }
        return Ok(());
    }

    let workspace_path = args.workspace.clone().unwrap_or(env::current_dir()?);
    println!();
    println!("发现可连接的本地 Agent");
    for row in &offer.rows {
        println!("  {row}");
    }
    for note in &offer.notes {
        println!("  {note}");
    }
    if agent_ferry_core::workspace::load(&paths)?
        .workspaces
        .is_empty()
    {
        println!("  默认运行位置: {}", workspace_path.display());
    }

    let confirmed = if args.yes {
        true
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        prompt_yes_default("连接以上 Agent？ [Y/n] ")?
    } else {
        println!("  运行 aferry activate --yes --workspace <目录> 可非交互连接");
        false
    };
    if !confirmed {
        println!("已跳过本地 Agent 连接；之后可重新运行 aferry activate");
        return Ok(());
    }

    if let Some(executable) = offer.claude {
        let diagnosis = agent_ferry_claude::bind(&paths, &executable)?;
        println!(
            "  ✓ Claude Code {}",
            diagnosis.version.as_deref().unwrap_or("")
        );
    }
    if let Some(executable) = offer.opencode {
        let diagnosis = agent_ferry_opencode::bind(&paths, &executable, DEFAULT_OPENCODE_MODEL)?;
        println!(
            "  ✓ OpenCode {}",
            diagnosis.version.as_deref().unwrap_or("")
        );
    }
    if let Some(executable) = offer.codex {
        let diagnosis = agent_ferry_codex::bind(&paths, &executable)?;
        println!("  ✓ Codex {}", diagnosis.version.as_deref().unwrap_or(""));
    }
    ensure_default_workspace(&paths, &workspace_path)?;
    println!("本地 Agent 已连接，重新打开 Chrome 扩展即可使用");
    Ok(())
}

fn inspect_unbound_local_agents(paths: &AgentFerryPaths) -> Result<LocalAgentOffer, CliError> {
    let mut offer = LocalAgentOffer::default();
    if !paths.claude_binding.exists() {
        let diagnosis = agent_ferry_claude::diagnose_binding(paths)?;
        let selected = if diagnosis.state == ClaudeState::Ready {
            diagnosis
                .executable
                .clone()
                .map(|path| (path, diagnosis.version))
        } else if diagnosis.state == ClaudeState::NeedsSelection {
            diagnosis.candidates.into_iter().find_map(|candidate| {
                let candidate_diagnosis = agent_ferry_claude::diagnose_executable(&candidate);
                (candidate_diagnosis.state == ClaudeState::Ready)
                    .then_some((candidate, candidate_diagnosis.version))
            })
        } else {
            None
        };
        if let Some((executable, version)) = selected {
            offer.rows.push(format!(
                "Claude Code  {}  {}",
                version.as_deref().unwrap_or("版本未知"),
                executable.display()
            ));
            offer.claude = Some(executable);
        } else if diagnosis.state == ClaudeState::NeedsSelection {
            offer
                .notes
                .push("Claude Code：发现了候选版本，但没有可直接连接的版本".to_owned());
        }
    }
    if !paths.opencode_binding.exists() {
        let diagnosis = agent_ferry_opencode::diagnose_binding(paths)?;
        let selected = if diagnosis.state == OpenCodeState::Ready {
            diagnosis
                .executable
                .clone()
                .map(|path| (path, diagnosis.version))
        } else if diagnosis.state == OpenCodeState::NeedsSelection {
            diagnosis.candidates.into_iter().find_map(|candidate| {
                let candidate_diagnosis =
                    agent_ferry_opencode::diagnose_executable(&candidate, DEFAULT_OPENCODE_MODEL);
                (candidate_diagnosis.state == OpenCodeState::Ready)
                    .then_some((candidate, candidate_diagnosis.version))
            })
        } else {
            None
        };
        if let Some((executable, version)) = selected {
            offer.rows.push(format!(
                "OpenCode     {}  {}",
                version.as_deref().unwrap_or("版本未知"),
                executable.display()
            ));
            offer.opencode = Some(executable);
        } else if diagnosis.state == OpenCodeState::NeedsSelection {
            offer
                .notes
                .push("OpenCode：发现了候选版本，但没有可直接连接的版本".to_owned());
        }
    }
    if !paths.codex_binding.exists() {
        let diagnosis = agent_ferry_codex::diagnose_binding(paths)?;
        let selected = if diagnosis.state == CodexState::Ready {
            diagnosis
                .executable
                .clone()
                .map(|path| (path, diagnosis.version))
        } else if diagnosis.state == CodexState::NeedsSelection {
            diagnosis.candidates.into_iter().find_map(|candidate| {
                let candidate_diagnosis = agent_ferry_codex::diagnose_executable(&candidate);
                (candidate_diagnosis.state == CodexState::Ready)
                    .then_some((candidate, candidate_diagnosis.version))
            })
        } else {
            None
        };
        if let Some((executable, version)) = selected {
            offer.rows.push(format!(
                "Codex        {}  {}",
                version.as_deref().unwrap_or("版本未知"),
                executable.display()
            ));
            offer.codex = Some(executable);
        } else if diagnosis.state == CodexState::NeedsSelection {
            offer
                .notes
                .push("Codex：发现了候选版本，但没有可直接连接的版本".to_owned());
        }
    }
    Ok(offer)
}

fn ensure_default_workspace(paths: &AgentFerryPaths, path: &Path) -> Result<(), CliError> {
    if !agent_ferry_core::workspace::load(paths)?
        .workspaces
        .is_empty()
    {
        return Ok(());
    }
    let canonical = path.canonicalize().map_err(|_| {
        agent_ferry_core::workspace::WorkspaceError::DirectoryMissing(path.to_owned())
    })?;
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("workspace");
    let workspace = agent_ferry_core::workspace::add(paths, name, &canonical)?;
    println!(
        "  ✓ 运行位置 {}  {}",
        workspace.name,
        workspace.path.display()
    );
    Ok(())
}

fn prompt_yes_default(prompt: &str) -> Result<bool, CliError> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

fn run_service_command(command: ServiceCommand) -> Result<i32, CliError> {
    let manager = service::ServiceManager::discover()?;
    match command {
        ServiceCommand::Install {
            daemon_path,
            output,
        } => {
            let report = manager.install(daemon_path.as_deref())?;
            print_service_report(&report, output.json)?;
            Ok(0)
        }
        ServiceCommand::Start(output) => {
            let report = manager.start()?;
            print_service_report(&report, output.json)?;
            Ok(0)
        }
        ServiceCommand::Stop(output) => {
            let report = manager.stop()?;
            print_service_report(&report, output.json)?;
            Ok(0)
        }
        ServiceCommand::Restart(output) => {
            let report = manager.restart()?;
            print_service_report(&report, output.json)?;
            Ok(0)
        }
        ServiceCommand::Status(output) => {
            let report = manager.status()?;
            let running = report.state != service::ServiceState::Stopped;
            print_service_report(&report, output.json)?;
            Ok(i32::from(!running))
        }
        ServiceCommand::Logs { lines } => {
            print!("{}", manager.logs(lines)?);
            Ok(0)
        }
        ServiceCommand::Uninstall(output) => {
            let report = manager.uninstall()?;
            print_service_report(&report, output.json)?;
            Ok(0)
        }
    }
}

fn print_service_report(report: &service::ServiceReport, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("Agent Ferry service: {:?}", report.state);
        if let Some(pid) = report.pid {
            println!("  PID: {pid}");
        }
        println!("  LaunchAgent: {}", report.plist.display());
        println!("  stdout: {}", report.stdout_log.display());
        println!("  stderr: {}", report.stderr_log.display());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct DataMigrationReport {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<PathBuf>,
}

fn run_data_command(command: DataCommand) -> Result<i32, CliError> {
    match command {
        DataCommand::Migrate(output) => {
            let home = env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or(agent_ferry_core::CoreError::HomeDirectoryUnavailable)?;
            let report = match migrate_legacy_data(&home)? {
                DataMigrationOutcome::NotNeeded => DataMigrationReport {
                    state: "not_needed",
                    from: None,
                    to: None,
                },
                DataMigrationOutcome::Migrated { from, to } => DataMigrationReport {
                    state: "migrated",
                    from: Some(from),
                    to: Some(to),
                },
            };
            if output.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if report.state == "migrated" {
                println!(
                    "已迁移 Agent Ferry 数据: {} -> {}",
                    report.from.as_ref().expect("迁移报告包含旧路径").display(),
                    report.to.as_ref().expect("迁移报告包含新路径").display()
                );
            } else {
                println!("无需迁移 Agent Ferry 数据");
            }
            Ok(0)
        }
    }
}

#[cfg(debug_assertions)]
fn run_dev_command(command: &DevCommand) -> Result<i32, CliError> {
    match command {
        DevCommand::CacheHermesCredentials => {
            let paths = AgentFerryPaths::discover()?;
            let source = KeychainCredentialStore;
            let destination =
                DevelopmentCredentialStore::new(paths.development_credentials.clone());
            let connections = load_connections(&paths.hermes_connections)?;
            let mut copied = 0usize;
            for connection in connections.connections {
                let secret = source
                    .get(&connection.credential_ref)?
                    .ok_or_else(|| CliError::MissingHermesCredential(connection.name.clone()))?;
                destination.set(&connection.credential_ref, &secret)?;
                copied += 1;
            }
            println!(
                "已缓存 {copied} 个 Hermes 开发凭据到 {}",
                paths.development_credentials.display()
            );
            Ok(0)
        }
    }
}

fn run_workspace_command(command: WorkspaceCommand) -> Result<i32, CliError> {
    let paths = AgentFerryPaths::discover()?;
    match command {
        WorkspaceCommand::Add { name, path, output } => {
            let workspace = agent_ferry_core::workspace::add(&paths, &name, &path)?;
            let diagnosis = diagnose_workspace(&workspace);
            print_workspace_diagnoses(&[diagnosis], output.json)?;
            Ok(0)
        }
        WorkspaceCommand::List(output) => {
            let config = agent_ferry_core::workspace::load(&paths)?;
            let diagnoses = config
                .workspaces
                .iter()
                .map(diagnose_workspace)
                .collect::<Vec<_>>();
            print_workspace_diagnoses(&diagnoses, output.json)?;
            Ok(0)
        }
        WorkspaceCommand::Doctor { identifier, output } => {
            let config = agent_ferry_core::workspace::load(&paths)?;
            let selected = config
                .workspaces
                .iter()
                .filter(|workspace| {
                    identifier
                        .as_ref()
                        .is_none_or(|value| workspace.id == *value || workspace.name == *value)
                })
                .map(diagnose_workspace)
                .collect::<Vec<_>>();
            if let Some(identifier) = identifier {
                if selected.is_empty() {
                    return Err(CliError::WorkspaceNotFound(identifier));
                }
            }
            let ready = selected
                .iter()
                .all(|diagnosis| diagnosis.state == WorkspaceState::Ready);
            print_workspace_diagnoses(&selected, output.json)?;
            Ok(i32::from(!ready))
        }
        WorkspaceCommand::Remove { identifier } => {
            let removed = agent_ferry_core::workspace::remove(&paths, &identifier)?;
            println!(
                "已移除 Workspace 配置: {} ({})；真实目录未删除",
                removed.name,
                removed.path.display()
            );
            Ok(0)
        }
    }
}

fn print_workspace_diagnoses(
    diagnoses: &[agent_ferry_core::workspace::WorkspaceDiagnosis],
    json: bool,
) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(diagnoses)?);
    } else if diagnoses.is_empty() {
        println!("尚未配置 Workspace");
    } else {
        for diagnosis in diagnoses {
            println!(
                "{}  {}  {:?}  {}",
                diagnosis.id,
                diagnosis.name,
                diagnosis.state,
                diagnosis.path.display()
            );
        }
    }
    Ok(())
}

fn run_agent_command(command: AgentCommand) -> Result<i32, CliError> {
    let paths = AgentFerryPaths::discover()?;
    match command {
        AgentCommand::Claude { command } => {
            let (diagnosis, output) = match command {
                ClaudeCommand::Detect(output) => {
                    (agent_ferry_claude::detect_and_auto_bind(&paths)?, output)
                }
                ClaudeCommand::Bind { path, output } => {
                    (agent_ferry_claude::bind(&paths, &path)?, output)
                }
                ClaudeCommand::Doctor(output) => {
                    (agent_ferry_claude::diagnose_binding(&paths)?, output)
                }
                ClaudeCommand::Run {
                    workspace,
                    document_file,
                    title,
                    source_url,
                    prompt_stdin,
                } => {
                    return run_claude_task(
                        &paths,
                        &workspace,
                        &document_file,
                        title,
                        source_url,
                        prompt_stdin,
                    );
                }
            };
            print_claude_diagnosis(&diagnosis, output.json)?;
            Ok(i32::from(diagnosis.state != ClaudeState::Ready))
        }
        AgentCommand::Opencode { command } => {
            let (diagnosis, output) = match command {
                OpenCodeCommand::Detect { model, output } => (
                    agent_ferry_opencode::detect_and_auto_bind(&paths, &model)?,
                    output,
                ),
                OpenCodeCommand::Bind {
                    path,
                    model,
                    output,
                } => (agent_ferry_opencode::bind(&paths, &path, &model)?, output),
                OpenCodeCommand::Doctor(output) => {
                    (agent_ferry_opencode::diagnose_binding(&paths)?, output)
                }
                OpenCodeCommand::Run {
                    workspace,
                    document_file,
                    title,
                    source_url,
                    prompt_stdin,
                } => {
                    return run_opencode_task(
                        &paths,
                        &workspace,
                        &document_file,
                        title,
                        source_url,
                        prompt_stdin,
                    );
                }
            };
            print_opencode_diagnosis(&diagnosis, output.json)?;
            Ok(i32::from(diagnosis.state != OpenCodeState::Ready))
        }
        AgentCommand::Codex { command } => run_codex_agent_command(&paths, command),
    }
}

fn run_codex_agent_command(
    paths: &AgentFerryPaths,
    command: CodexCommand,
) -> Result<i32, CliError> {
    let (diagnosis, output) = match command {
        CodexCommand::Detect(output) => (agent_ferry_codex::detect_and_auto_bind(paths)?, output),
        CodexCommand::Bind { path, output } => (agent_ferry_codex::bind(paths, &path)?, output),
        CodexCommand::Doctor(output) => (agent_ferry_codex::diagnose_binding(paths)?, output),
        CodexCommand::Run {
            surface,
            workspace,
            document_file,
            title,
            source_url,
            prompt_stdin,
        } => {
            return run_codex_task(
                paths,
                surface.into(),
                &workspace,
                &document_file,
                title,
                source_url,
                prompt_stdin,
            );
        }
    };
    print_codex_diagnosis(&diagnosis, output.json)?;
    Ok(i32::from(diagnosis.state != CodexState::Ready))
}

fn run_codex_task(
    paths: &AgentFerryPaths,
    surface: CodexSurface,
    workspace: &Path,
    document_file: &Path,
    title: String,
    source_url: String,
    prompt_stdin: bool,
) -> Result<i32, CliError> {
    if !prompt_stdin {
        return Err(CliError::CodexPromptStdinRequired);
    }
    let prompt = read_limited_input(io::stdin().lock())?;
    let markdown = read_limited_utf8_file(document_file, MAX_CODEX_DOCUMENT_BYTES).map_err(
        |error| match error {
            CliError::ClaudeDocumentTooLarge => CliError::CodexDocumentTooLarge,
            CliError::ClaudeDocumentNotUtf8 => CliError::CodexDocumentNotUtf8,
            other => other,
        },
    )?;
    let document = CodexDocument {
        title,
        source_url,
        markdown,
    };
    agent_ferry_codex::run_task(
        paths,
        surface,
        workspace,
        &prompt,
        &document,
        |event| match event {
            CodexTaskEvent::Started {
                thread_id,
                artifact,
            } => {
                println!("Codex Task 已启动: {thread_id}");
                println!("Artifact: {}", artifact.display());
            }
            CodexTaskEvent::Output(text) => print!("{text}"),
            CodexTaskEvent::Tool(text) => eprintln!("[codex tool] {text}"),
            CodexTaskEvent::Diagnostic(text) => eprintln!("[codex] {text}"),
            CodexTaskEvent::Completed(_) => println!("\nCodex Task 已完成"),
            CodexTaskEvent::Failed(text) => eprintln!("Codex Task 失败: {text}"),
        },
    )?;
    Ok(0)
}

fn run_opencode_task(
    paths: &AgentFerryPaths,
    workspace: &Path,
    document_file: &Path,
    title: String,
    source_url: String,
    prompt_stdin: bool,
) -> Result<i32, CliError> {
    if !prompt_stdin {
        return Err(CliError::OpenCodePromptStdinRequired);
    }
    let prompt = read_limited_input(io::stdin().lock())?;
    let markdown =
        read_limited_utf8_file(document_file, MAX_OPENCODE_DOCUMENT_BYTES).map_err(|error| {
            match error {
                CliError::ClaudeDocumentTooLarge => CliError::OpenCodeDocumentTooLarge,
                CliError::ClaudeDocumentNotUtf8 => CliError::OpenCodeDocumentNotUtf8,
                other => other,
            }
        })?;
    let document = OpenCodeDocument {
        title,
        source_url,
        markdown,
    };
    agent_ferry_opencode::run_task(paths, workspace, &prompt, &document, |event| match event {
        OpenCodeTaskEvent::Started { task_id, artifact } => {
            println!("OpenCode Task 已启动: {task_id}");
            println!("Artifact: {}", artifact.display());
        }
        OpenCodeTaskEvent::Output(text) => print!("{text}"),
        OpenCodeTaskEvent::Tool(text) => eprintln!("[opencode tool] {text}"),
        OpenCodeTaskEvent::Diagnostic(text) => eprintln!("[opencode] {text}"),
        OpenCodeTaskEvent::Completed(_) => println!("\nOpenCode Task 已完成"),
        OpenCodeTaskEvent::Failed(text) => eprintln!("OpenCode Task 失败: {text}"),
    })?;
    Ok(0)
}

fn run_claude_task(
    paths: &AgentFerryPaths,
    workspace: &Path,
    document_file: &Path,
    title: String,
    source_url: String,
    prompt_stdin: bool,
) -> Result<i32, CliError> {
    if !prompt_stdin {
        return Err(CliError::ClaudePromptStdinRequired);
    }
    let prompt = read_limited_input(io::stdin().lock())?;
    let markdown = read_limited_utf8_file(document_file, MAX_CLAUDE_DOCUMENT_BYTES)?;
    let document = ClaudeDocument {
        title,
        source_url,
        markdown,
    };
    agent_ferry_claude::run_print_task(
        paths,
        workspace,
        &prompt,
        &document,
        |event| match event {
            ClaudeTaskEvent::Started {
                session_id,
                artifact,
            } => {
                println!("Claude Task 已启动: {session_id}");
                println!("Artifact: {}", artifact.display());
            }
            ClaudeTaskEvent::Output(text) => print!("{text}"),
            ClaudeTaskEvent::Diagnostic(text) => eprintln!("[claude] {text}"),
            ClaudeTaskEvent::Completed(_) => println!("\nClaude Task 已完成"),
            ClaudeTaskEvent::Failed(text) => eprintln!("Claude Task 失败: {text}"),
        },
    )?;
    Ok(0)
}

fn read_limited_utf8_file(path: &Path, limit: usize) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(CliError::ClaudeDocumentTooLarge);
    }
    String::from_utf8(bytes).map_err(|_| CliError::ClaudeDocumentNotUtf8)
}

fn print_claude_diagnosis(diagnosis: &ClaudeDiagnosis, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(diagnosis)?);
        return Ok(());
    }
    println!("Claude Code  {}", claude_state_name(diagnosis.state));
    println!("  {}", diagnosis.detail);
    if let Some(executable) = &diagnosis.executable {
        println!("  executable: {}", executable.display());
    }
    if let Some(version) = &diagnosis.version {
        println!("  version: {version}");
    }
    for candidate in &diagnosis.candidates {
        println!("  candidate: {}", candidate.display());
    }
    if diagnosis.state == ClaudeState::NotDetected {
        println!("  官方安装文档: https://code.claude.com/docs/en/installation");
        println!("  安装完成后复检: aferry agent claude detect");
    } else if diagnosis.state == ClaudeState::NeedsSelection {
        println!("  选择命令: aferry agent claude bind --path <absolute-path>");
    } else if diagnosis.state == ClaudeState::NotAuthenticated {
        println!("  请先运行 Claude Code 官方登录流程，再执行 aferry agent claude doctor");
    }
    Ok(())
}

fn print_opencode_diagnosis(diagnosis: &OpenCodeDiagnosis, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(diagnosis)?);
    } else {
        println!("OpenCode: {:?}", diagnosis.state);
        println!("  {}", diagnosis.detail);
        if let Some(executable) = &diagnosis.executable {
            println!("  executable: {}", executable.display());
        }
        if let Some(version) = &diagnosis.version {
            println!("  version: {version}");
        }
        if let Some(model) = &diagnosis.model {
            println!("  model: {model}");
        }
        if diagnosis.state == OpenCodeState::NotDetected {
            println!("  安装说明: https://opencode.ai/docs/");
            println!("  安装完成后: aferry agent opencode detect");
        }
    }
    Ok(())
}

fn print_codex_diagnosis(diagnosis: &CodexDiagnosis, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", serde_json::to_string_pretty(diagnosis)?);
        return Ok(());
    }
    println!("Codex: {:?}", diagnosis.state);
    println!("  {}", diagnosis.detail);
    if let Some(executable) = &diagnosis.executable {
        println!("  executable: {}", executable.display());
    }
    if let Some(version) = &diagnosis.version {
        println!("  version: {version}");
    }
    println!("  Codex CLI: {}", diagnosis.cli_supported);
    println!("  Codex App Server: {}", diagnosis.app_server_supported);
    for candidate in &diagnosis.candidates {
        println!("  candidate: {}", candidate.display());
    }
    if diagnosis.state == CodexState::NotDetected {
        println!("  Agent Ferry 不会代为安装 Codex；请先安装官方 CLI 或桌面 App");
    } else if diagnosis.state == CodexState::NeedsSelection {
        println!("  选择命令: aferry agent codex bind --path <absolute-path>");
    } else if diagnosis.state == CodexState::NotAuthenticated {
        println!("  请先运行 codex login，再执行 aferry agent codex doctor");
    }
    Ok(())
}

const fn claude_state_name(state: ClaudeState) -> &'static str {
    match state {
        ClaudeState::NotDetected => "not_detected",
        ClaudeState::NeedsSelection => "needs_selection",
        ClaudeState::Incompatible => "incompatible",
        ClaudeState::NotAuthenticated => "not_authenticated",
        ClaudeState::Ready => "ready",
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
            Vec::new().into_boxed_slice(),
        ),
    };

    let mut next_actions = Vec::new();
    if daemon.state != CheckState::Ready {
        next_actions.push(
            "运行 aferry service status；未安装时执行 aferry service install，然后重新运行 aferry doctor"
                .to_owned(),
        );
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
        next_actions.push("运行 aferry connect hermes <user@host> 快速连接远程 Hermes".to_owned());
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
        ConnectionCommand::Setup {
            kind:
                ConnectionSetupKind::Hermes {
                    name,
                    ssh_host,
                    container,
                    yes,
                },
        } => setup_docker_hermes(&paths, &name, &ssh_host, &container, yes),
        ConnectionCommand::Add {
            kind:
                ConnectionKind::Hermes {
                    name,
                    url,
                    model,
                    ssh_host,
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
                    transport: ssh_host.map_or(ConnectionTransportConfig::Direct, |ssh_host| {
                        ConnectionTransportConfig::SshTunnel { ssh_host }
                    }),
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
        ConnectionCommand::List(output) => list_connections(&paths, &output),
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
        ConnectionCommand::Run {
            identifier,
            input_file,
            input_stdin,
        } => run_hermes_input(&paths, &identifier, input_file.as_deref(), input_stdin),
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

fn run_connect_command(command: ConnectCommand) -> Result<i32, CliError> {
    let paths = AgentFerryPaths::discover()?;
    match command {
        ConnectCommand::Hermes {
            ssh_host,
            name,
            yes,
        } => {
            let name = name.unwrap_or_else(|| default_hermes_connection_name(&ssh_host));
            setup_docker_hermes(&paths, &name, &ssh_host, "hermes", yes)
        }
    }
}

fn default_hermes_connection_name(ssh_host: &str) -> String {
    let host = ssh_host.rsplit_once('@').map_or(ssh_host, |(_, host)| host);
    let label = host
        .trim_matches(['[', ']'])
        .split(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or("remote");
    let normalized = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("{normalized}-hermes")
}

fn run_hermes_input(
    paths: &AgentFerryPaths,
    identifier: &str,
    input_file: Option<&Path>,
    input_stdin: bool,
) -> Result<i32, CliError> {
    if input_file.is_none() && !input_stdin {
        return Err(CliError::RunInputRequired);
    }
    let input = if let Some(path) = input_file {
        read_limited_input(fs::File::open(path)?)?
    } else {
        read_limited_input(io::stdin().lock())?
    };
    let connections = load_connections(&paths.hermes_connections)?;
    let connection = connections
        .connections
        .into_iter()
        .find(|connection| connection.id == identifier || connection.name == identifier)
        .ok_or_else(|| CliError::ConnectionNotFound(identifier.to_owned()))?;
    let task_id = format!("cli-{}", Uuid::new_v4().simple());
    let request = HostRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        command: Command::HermesRun {
            task_id,
            target_id: connection.id,
            input,
        },
    };
    let mut stream = open_ipc_stream(paths, ConnectorKind::Cli, serde_json::to_value(request)?)?;
    observe_hermes_run(&mut stream)
}

fn read_limited_input(reader: impl Read) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(MAX_HERMES_RUN_INPUT_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_HERMES_RUN_INPUT_BYTES {
        return Err(CliError::RunInputTooLarge);
    }
    let input = String::from_utf8(bytes).map_err(|_| CliError::RunInputNotUtf8)?;
    if input.trim().is_empty() {
        return Err(CliError::RunInputEmpty);
    }
    Ok(input)
}

fn observe_hermes_run(stream: &mut std::os::unix::net::UnixStream) -> Result<i32, CliError> {
    loop {
        let value: serde_json::Value = match read_json_frame(stream) {
            Ok(value) => value,
            Err(FrameError::EndOfStream) => return Err(CliError::RunEndedBeforeTerminal),
            Err(error) => return Err(error.into()),
        };
        if value.get("event").is_none() {
            let response: HostResponse = serde_json::from_value(value)?;
            return match response.outcome {
                ResponseOutcome::Success { .. } => Err(CliError::RunEndedBeforeTerminal),
                ResponseOutcome::Failure { error } => Err(CliError::DaemonRejected(error.message)),
            };
        }
        let event: HandoffEvent = serde_json::from_value(value)?;
        match event.event {
            HandoffEventKind::Submitted => {
                println!(
                    "Hermes Run 已提交: {}",
                    event.run_id.as_deref().unwrap_or("等待 run_id")
                );
            }
            HandoffEventKind::Running => println!("Hermes Run 执行中"),
            HandoffEventKind::OutputDelta => {
                if let Some(text) = event.text {
                    println!("[output] {text}");
                }
            }
            HandoffEventKind::ToolStarted => {
                println!(
                    "[tool:start] {}",
                    event.text.as_deref().unwrap_or("unknown")
                );
            }
            HandoffEventKind::ToolCompleted => {
                println!("[tool:done] {}", event.text.as_deref().unwrap_or("unknown"));
            }
            HandoffEventKind::Completed => {
                if let Some(text) = event.text {
                    println!("[result] {text}");
                }
                println!("Hermes Run 已完成");
                return Ok(0);
            }
            HandoffEventKind::Failed => {
                if let Some(text) = event.text {
                    eprintln!("Hermes Run 失败: {text}");
                } else {
                    eprintln!("Hermes Run 失败");
                }
                return Ok(1);
            }
            HandoffEventKind::Cancelled => {
                eprintln!("Hermes Run 已取消");
                return Ok(2);
            }
        }
    }
}

fn setup_docker_hermes(
    paths: &AgentFerryPaths,
    name: &str,
    ssh_host: &str,
    container: &str,
    yes: bool,
) -> Result<i32, CliError> {
    if name.is_empty()
        || name.len() > 128
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(CliError::InvalidConnectionName);
    }
    let existing = load_connections(&paths.hermes_connections)?
        .connections
        .into_iter()
        .find(|connection| connection.name == name);
    if let Some(existing) = existing {
        let same_route = existing.endpoint.base_url.as_str() == "http://127.0.0.1:8642/"
            && matches!(
                existing.transport,
                agent_ferry_hermes::HermesTransport::SshTunnel { ssh_host: ref configured }
                    if configured == ssh_host
            );
        if !same_route {
            return Err(CliError::ConnectionAlreadyExists(name.to_owned()));
        }
        let result = send_daemon_command(paths, Command::Status)?;
        let target = result
            .targets
            .iter()
            .find(|target| target.id == existing.id)
            .ok_or_else(|| CliError::ConnectionNotFound(name.to_owned()))?;
        if target.state != HandoffTargetState::Ready {
            return Err(CliError::ExistingConnectionNotReady(
                target_state_detail(target.state).to_owned(),
            ));
        }
        println!("Hermes Connection 已准备且验证通过: {name}");
        if !target.capabilities.is_empty() {
            println!("  capabilities: {}", target.capabilities.join(", "));
        }
        return Ok(0);
    }

    // 远端变更前确认 daemon 可用，避免服务器准备成功后本机却无法保存 Connection。
    send_daemon_command(paths, Command::Status)?;
    let runner = hermes_setup::SshHermesSetup::system();
    let preflight = runner.inspect(ssh_host, container)?;
    println!("Hermes Docker 准备计划");
    println!("  SSH Host:    {ssh_host}");
    println!("  容器:        {}", preflight.container);
    println!("  数据目录:    {} → /opt/data", preflight.data_source);
    println!("  镜像:        {}", preflight.image);
    if preflight.ready {
        println!("  状态:        远端 API 已准备，将复用现有配置");
    } else {
        println!("  变更:        保留旧容器为 {}", preflight.backup_container);
        println!("  变更:        新增 127.0.0.1:8642 → 容器 8642");
        println!("  变更:        生成 API Key；不公开、不写入本机文件");
    }

    if !yes {
        if !io::stdin().is_terminal() {
            return Err(CliError::ConfirmationRequired);
        }
        print!("继续执行？[y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
            println!("已取消，远端未修改");
            return Ok(0);
        }
    }

    let prepared = runner.apply(ssh_host, container)?;
    let result = send_daemon_command(
        paths,
        Command::ConnectionAdd {
            name: name.to_owned(),
            base_url: "http://127.0.0.1:8642".to_owned(),
            model: None,
            transport: ConnectionTransportConfig::SshTunnel {
                ssh_host: ssh_host.to_owned(),
            },
            token: prepared.into_token(),
        },
    )?;
    let target = result
        .targets
        .iter()
        .find(|target| target.name == name)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_owned()))?;
    if target.state != HandoffTargetState::Ready {
        return Err(CliError::PreparedConnectionNotReady(
            target_state_detail(target.state).to_owned(),
        ));
    }
    println!("已准备 Hermes 并完成 Connection 验证: {name}");
    if !target.capabilities.is_empty() {
        println!("  capabilities: {}", target.capabilities.join(", "));
    }
    Ok(0)
}

fn list_connections(paths: &AgentFerryPaths, output: &OutputArgs) -> Result<i32, CliError> {
    let connections = load_connections(&paths.hermes_connections)?;
    let mut items = Vec::with_capacity(connections.connections.len());
    for connection in connections.connections {
        let (transport, ssh_host) = match connection.transport {
            agent_ferry_hermes::HermesTransport::Direct => ("direct", None),
            agent_ferry_hermes::HermesTransport::SshTunnel { ssh_host } => {
                ("ssh_tunnel", Some(ssh_host))
            }
        };
        items.push(ConnectionListItem {
            id: connection.id,
            name: connection.name,
            kind: "remote_hermes",
            transport,
            ssh_host,
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
            let route = item.ssh_host.map_or_else(
                || item.transport.to_owned(),
                |ssh_host| format!("{}:{ssh_host}", item.transport),
            );
            println!("{}  {}  {}  {}", item.id, item.name, route, item.endpoint);
        }
    }
    Ok(0)
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
    if let Some(identifier) = identifier {
        if selected.is_empty() {
            return Err(CliError::ConnectionNotFound(identifier.to_owned()));
        }
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
    Frame(#[from] FrameError),
    #[error(transparent)]
    Hermes(#[from] agent_ferry_hermes::HermesError),
    #[error(transparent)]
    Claude(#[from] agent_ferry_claude::ClaudeError),
    #[error(transparent)]
    OpenCode(#[from] agent_ferry_opencode::OpenCodeError),
    #[error(transparent)]
    Codex(#[from] agent_ferry_codex::CodexError),
    #[error(transparent)]
    Workspace(#[from] agent_ferry_core::workspace::WorkspaceError),
    #[error(transparent)]
    Service(#[from] service::ServiceError),
    #[error(transparent)]
    Update(#[from] update::UpdateError),
    #[error(transparent)]
    Uninstall(#[from] uninstall::UninstallError),
    #[error("未找到 Workspace: {0}")]
    WorkspaceNotFound(String),
    #[error("请显式传入 --prompt-stdin，Prompt 不允许进入 argv")]
    ClaudePromptStdinRequired,
    #[error("Claude 文档超过 8 MiB 上限")]
    ClaudeDocumentTooLarge,
    #[error("Claude 文档必须是 UTF-8")]
    ClaudeDocumentNotUtf8,
    #[error("请显式传入 --prompt-stdin，Prompt 不允许进入 argv")]
    OpenCodePromptStdinRequired,
    #[error("OpenCode 文档超过 8 MiB 上限")]
    OpenCodeDocumentTooLarge,
    #[error("OpenCode 文档必须是 UTF-8")]
    OpenCodeDocumentNotUtf8,
    #[error("请显式传入 --prompt-stdin，Prompt 不允许进入 argv")]
    CodexPromptStdinRequired,
    #[error("Codex 文档超过 8 MiB 上限")]
    CodexDocumentTooLarge,
    #[error("Codex 文档必须是 UTF-8")]
    CodexDocumentNotUtf8,
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
    #[cfg(debug_assertions)]
    #[error("Hermes Connection 缺少钥匙串凭据: {0}")]
    MissingHermesCredential(String),
    #[error("请使用 --input-file 或 --input-stdin 提供 Hermes Run input")]
    RunInputRequired,
    #[error("Hermes Run input 不能为空")]
    RunInputEmpty,
    #[error("Hermes Run input 必须是 UTF-8")]
    RunInputNotUtf8,
    #[error("Hermes Run input 超过 512 KiB 上限")]
    RunInputTooLarge,
    #[error("Hermes Run 连接在终态前结束")]
    RunEndedBeforeTerminal,
    #[error("未找到 Hermes Connection: {0}")]
    ConnectionNotFound(String),
    #[error("Hermes Connection 已存在: {0}；请先运行 connection doctor 或 remove")]
    ConnectionAlreadyExists(String),
    #[error("Connection 名称不能为空、不能包含首尾空白或控制字符，且最多 128 字节")]
    InvalidConnectionName,
    #[error("相同 Hermes Connection 已存在但诊断失败: {0}；请运行 connection doctor")]
    ExistingConnectionNotReady(String),
    #[error("需要交互确认；非交互执行时请在审阅计划后显式传入 --yes")]
    ConfirmationRequired,
    #[error("远端已准备，但 Connection 验证失败: {0}")]
    PreparedConnectionNotReady(String),
    #[error(transparent)]
    HermesSetup(#[from] hermes_setup::HermesSetupError),
    #[error("agentferryd 拒绝命令: {0}")]
    DaemonRejected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_hermes_name_hides_ssh_details() {
        assert_eq!(
            default_hermes_connection_name("root@ktoon.site"),
            "ktoon-hermes"
        );
        assert_eq!(
            default_hermes_connection_name("home-server"),
            "home-server-hermes"
        );
    }

    #[test]
    fn quick_hermes_command_accepts_only_the_server_by_default() {
        let cli = Cli::try_parse_from(["aferry", "connect", "hermes", "root@ktoon.site"])
            .expect("解析 Hermes 快速连接");
        let Some(CliCommand::Connect {
            command:
                ConnectCommand::Hermes {
                    ssh_host,
                    name,
                    yes,
                },
        }) = cli.command
        else {
            panic!("应解析为 Hermes 快速连接");
        };
        assert_eq!(ssh_host, "root@ktoon.site");
        assert_eq!(name, None);
        assert!(!yes);
    }
}
