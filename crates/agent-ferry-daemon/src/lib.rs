use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_ferry_core::{AgentFerryPaths, load_or_create_connector_token};
use agent_ferry_protocol::{
    Command, ConnectorKind, ErrorCode, HostRequest, HostResponse, IpcEnvelope, MAX_MESSAGE_BYTES,
    PROTOCOL_VERSION, ServiceState, StatusResult,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

#[derive(Debug)]
pub struct Daemon {
    paths: AgentFerryPaths,
    listener: UnixListener,
    auth_token: Arc<str>,
    chrome_seen: Arc<AtomicBool>,
}

impl Daemon {
    /// 绑定私有 Unix Socket，并拒绝覆盖仍存活的 daemon。
    ///
    /// # Errors
    ///
    /// token 初始化、旧 socket 检查、绑定或权限设置失败时返回错误。
    pub fn bind(paths: AgentFerryPaths) -> io::Result<Self> {
        let auth_token = load_or_create_connector_token(&paths).map_err(io::Error::other)?;
        if paths.socket.exists() {
            match StdUnixStream::connect(&paths.socket) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("daemon 已经在 {} 监听", paths.socket.display()),
                    ));
                }
                Err(_) => {
                    // 上次异常退出会遗留 socket 文件；只有确认无法连接后才清理，
                    // 避免第二个 daemon 抢占仍然存活的实例。
                    std::fs::remove_file(&paths.socket)?;
                }
            }
        }

        let listener = UnixListener::bind(&paths.socket)?;
        std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            paths,
            listener,
            auth_token: Arc::from(auth_token),
            chrome_seen: Arc::new(AtomicBool::new(false)),
        })
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.paths.socket
    }

    /// 持续处理 Connector 请求，直到收到停止信号。
    ///
    /// # Errors
    ///
    /// socket accept 或退出时清理 socket 失败时返回错误。
    pub async fn serve_until<F>(self, shutdown: F) -> io::Result<()>
    where
        F: Future<Output = ()>,
    {
        info!(socket = %self.paths.socket.display(), "agentferryd 已启动");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => {
                    info!("agentferryd 收到停止信号");
                    break;
                }
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let token = Arc::clone(&self.auth_token);
                    let chrome_seen = Arc::clone(&self.chrome_seen);
                    tokio::spawn(async move {
                        if let Err(error) = handle_connection(stream, &token, &chrome_seen).await {
                            warn!(error = %error, "本地 Connector 请求失败");
                        }
                    });
                }
            }
        }
        if self.paths.socket.exists() {
            std::fs::remove_file(&self.paths.socket)?;
        }
        Ok(())
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    expected_token: &str,
    chrome_seen: &AtomicBool,
) -> io::Result<()> {
    let payload = match read_async_frame(&mut stream).await {
        Ok(payload) => payload,
        Err(error) => {
            let response = HostResponse::failure(
                "unknown",
                frame_error_code(&error),
                error.to_string(),
                false,
            );
            let _ = write_async_json(&mut stream, &response).await;
            return Ok(());
        }
    };

    let envelope: IpcEnvelope = match serde_json::from_slice(&payload) {
        Ok(envelope) => envelope,
        Err(error) => {
            let response = HostResponse::failure(
                "unknown",
                ErrorCode::InvalidMessage,
                format!("IPC envelope 不是有效 JSON: {error}"),
                false,
            );
            write_async_json(&mut stream, &response).await?;
            return Ok(());
        }
    };

    let request_id = request_id_from_value(&envelope.request);
    if envelope.auth_token != expected_token {
        warn!(request_id, connector = ?envelope.connector, "拒绝无效 Connector token");
        let response = HostResponse::failure(
            &request_id,
            ErrorCode::AuthenticationFailed,
            "Connector 身份校验失败",
            false,
        );
        write_async_json(&mut stream, &response).await?;
        return Ok(());
    }

    if envelope.connector == ConnectorKind::ChromeNativeHost {
        chrome_seen.store(true, Ordering::Release);
    }
    let response = dispatch_request(envelope.request, &envelope.connector, chrome_seen);
    write_async_json(&mut stream, &response).await
}

fn dispatch_request(
    value: Value,
    connector: &ConnectorKind,
    chrome_seen: &AtomicBool,
) -> HostResponse {
    let request_id = request_id_from_value(&value);
    let Some(protocol_version) = value.get("protocol_version").and_then(Value::as_u64) else {
        return HostResponse::failure(
            &request_id,
            ErrorCode::InvalidMessage,
            "缺少 protocol_version",
            false,
        );
    };
    if protocol_version != u64::from(PROTOCOL_VERSION) {
        return HostResponse::failure(
            &request_id,
            ErrorCode::ProtocolVersionUnsupported,
            format!("不支持协议版本 {protocol_version}，当前版本为 {PROTOCOL_VERSION}"),
            false,
        );
    }

    let command_type = value
        .get("command")
        .and_then(|command| command.get("type"))
        .and_then(Value::as_str);
    if command_type != Some("status") {
        return HostResponse::failure(
            &request_id,
            ErrorCode::UnknownCommand,
            format!("未知命令 {}", command_type.unwrap_or("<missing>")),
            false,
        );
    }

    let request: HostRequest = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(error) => {
            return HostResponse::failure(
                &request_id,
                ErrorCode::InvalidMessage,
                format!("请求结构无效: {error}"),
                false,
            );
        }
    };

    info!(
        request_id = request.request_id,
        connector = ?connector,
        command = ?request.command,
        "处理本地 Connector 请求"
    );
    match request.command {
        Command::Status => HostResponse::success(
            request.request_id,
            StatusResult {
                core_version: env!("CARGO_PKG_VERSION").to_owned(),
                daemon: ServiceState::Ready,
                native_host: if *connector == ConnectorKind::ChromeNativeHost
                    || chrome_seen.load(Ordering::Acquire)
                {
                    ServiceState::Ready
                } else {
                    ServiceState::NotDetected
                },
                chrome_extension: if chrome_seen.load(Ordering::Acquire) {
                    ServiceState::Ready
                } else {
                    ServiceState::NotDetected
                },
                capabilities: vec!["target.read".to_owned()],
            },
        ),
    }
}

fn request_id_from_value(value: &Value) -> String {
    value
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

async fn read_async_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let length = stream.read_u32_le().await? as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("消息长度 {length} 超过上限 {MAX_MESSAGE_BYTES}"),
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

async fn write_async_json(stream: &mut UnixStream, response: &HostResponse) -> io::Result<()> {
    let payload = serde_json::to_vec(response).map_err(io::Error::other)?;
    let length = u32::try_from(payload.len()).map_err(io::Error::other)?;
    stream.write_u32_le(length).await?;
    stream.write_all(&payload).await?;
    stream.shutdown().await
}

fn frame_error_code(error: &io::Error) -> ErrorCode {
    if error.kind() == io::ErrorKind::InvalidData {
        ErrorCode::MessageTooLarge
    } else {
        ErrorCode::InvalidMessage
    }
}
