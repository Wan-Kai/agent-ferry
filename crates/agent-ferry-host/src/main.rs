use std::io::{self, Write};

use agent_ferry_core::{AgentFerryPaths, open_ipc_stream};
use agent_ferry_protocol::{
    ConnectorKind, ErrorCode, FrameError, HostResponse, read_frame, write_json_frame,
};
use serde_json::Value;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        // Native Messaging 的 stdout 只能写协议帧，任何日志都必须留在 stderr。
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    info!("Native Host 已启动");
    if let Err(error) = run() {
        error!(error = %error, "Native Host 异常退出");
        std::process::exit(1);
    }
    info!("Native Host 已退出");
}

fn run() -> Result<(), HostError> {
    let paths = AgentFerryPaths::discover()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    loop {
        let payload = match read_frame(&mut reader) {
            Ok(payload) => payload,
            Err(FrameError::EndOfStream) => {
                info!("Chrome 已关闭 Native Messaging 连接");
                return Ok(());
            }
            Err(error) => {
                let response = HostResponse::failure(
                    "unknown",
                    frame_error_code(&error),
                    error.to_string(),
                    false,
                );
                write_json_frame(&mut writer, &response)?;
                return Ok(());
            }
        };

        let request: Value = match serde_json::from_slice(&payload) {
            Ok(request) => request,
            Err(error) => {
                warn!(error = %error, "Chrome 发送了无效 JSON");
                let response = HostResponse::failure(
                    "unknown",
                    ErrorCode::InvalidMessage,
                    format!("消息不是有效 JSON: {error}"),
                    false,
                );
                write_json_frame(&mut writer, &response)?;
                continue;
            }
        };
        let request_id = request
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();

        let mut daemon = match open_ipc_stream(&paths, ConnectorKind::ChromeNativeHost, request) {
            Ok(stream) => stream,
            Err(error) => {
                warn!(request_id, error = %error, "无法连接 agentferryd");
                let response = HostResponse::failure(
                    request_id,
                    ErrorCode::DaemonUnavailable,
                    "无法连接 agentferryd，请先运行 aferry setup 查看修复命令",
                    true,
                );
                write_json_frame(&mut writer, &response)?;
                writer.flush()?;
                continue;
            }
        };
        loop {
            match read_frame(&mut daemon) {
                Ok(message) => {
                    agent_ferry_protocol::write_frame(&mut writer, &message)?;
                    writer.flush()?;
                }
                Err(FrameError::EndOfStream) => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn frame_error_code(error: &FrameError) -> ErrorCode {
    match error {
        FrameError::MessageTooLarge { .. } => ErrorCode::MessageTooLarge,
        FrameError::EndOfStream | FrameError::Io(_) | FrameError::InvalidJson(_) => {
            ErrorCode::InvalidMessage
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Io(#[from] io::Error),
}
