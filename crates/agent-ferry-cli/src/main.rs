use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent_ferry_core::{
    AgentFerryPaths, NativeHostManifest, read_native_host_manifest, send_ipc_request,
};
use agent_ferry_protocol::{
    Command, ConnectorKind, HostRequest, PROTOCOL_VERSION, ResponseOutcome, ServiceState,
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

#[derive(Debug, Clone, Serialize)]
struct SetupReport {
    core: Check,
    daemon: Check,
    native_host: Check,
    chrome_extension: Check,
    next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Check {
    state: CheckState,
    detail: String,
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
    }
}

fn collect_report() -> Result<SetupReport, CliError> {
    let paths = AgentFerryPaths::discover()?;
    let native_host = inspect_native_host(&paths);
    let request = HostRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        command: Command::Status,
    };
    let daemon_response = serde_json::to_value(request)
        .ok()
        .and_then(|request| send_ipc_request(&paths, ConnectorKind::Cli, request).ok());

    let (daemon, chrome_extension) = match daemon_response {
        Some(response) => match response.outcome {
            ResponseOutcome::Success { result } => (
                Check {
                    state: CheckState::Ready,
                    detail: format!("agentferryd {} 已连接", result.core_version),
                },
                service_check(&result.chrome_extension, "Chrome 扩展已连接过 Native Host"),
            ),
            ResponseOutcome::Failure { error } => (
                Check {
                    state: CheckState::Broken,
                    detail: format!("daemon 返回 {:?}: {}", error.code, error.message),
                },
                Check {
                    state: CheckState::NotDetected,
                    detail: "尚未从 daemon 确认 Chrome 扩展".to_owned(),
                },
            ),
        },
        None => (
            Check {
                state: CheckState::NotDetected,
                detail: "无法连接 agentferryd".to_owned(),
            },
            Check {
                state: CheckState::NotDetected,
                detail: "daemon 不可用，无法确认扩展连接".to_owned(),
            },
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

    Ok(SetupReport {
        core: Check {
            state: CheckState::Ready,
            detail: format!("aferry {}", env!("CARGO_PKG_VERSION")),
        },
        daemon,
        native_host,
        chrome_extension,
        next_actions,
    })
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
    #[error("Chrome extension id 必须是 32 个 a-p 小写字符")]
    InvalidExtensionId,
    #[error("Native Host 不可执行: {0}")]
    HostNotExecutable(PathBuf),
}
