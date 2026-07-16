use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agent_ferry_core::{AgentFerryPaths, load_or_create_connector_token};
use agent_ferry_hermes::{
    DiagnosisState, HermesClient, HermesConnection, KeychainCredentialStore, add_connection,
    load_connections, remove_connection,
};
use agent_ferry_protocol::{
    Command, ConnectorKind, ErrorCode, HandoffTargetKind, HandoffTargetState, HandoffTargetStatus,
    HostRequest, HostResponse, IpcEnvelope, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ServiceState,
    StatusResult,
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
    hermes_client: Arc<HermesClient>,
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
        let hermes_client = HermesClient::new(Duration::from_secs(5)).map_err(io::Error::other)?;
        Ok(Self {
            paths,
            listener,
            auth_token: Arc::from(auth_token),
            chrome_seen: Arc::new(AtomicBool::new(false)),
            hermes_client: Arc::new(hermes_client),
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
                    let paths = self.paths.clone();
                    let hermes_client = Arc::clone(&self.hermes_client);
                    tokio::spawn(async move {
                        if let Err(error) = handle_connection(
                            stream,
                            &token,
                            &chrome_seen,
                            &paths,
                            &hermes_client,
                        ).await {
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
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
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
    let response = dispatch_request(
        envelope.request,
        &envelope.connector,
        chrome_seen,
        paths,
        hermes_client,
    )
    .await;
    write_async_json(&mut stream, &response).await
}

async fn dispatch_request(
    value: Value,
    connector: &ConnectorKind,
    chrome_seen: &AtomicBool,
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
) -> HostResponse {
    let request = match decode_request(value) {
        Ok(request) => request,
        Err(response) => return response,
    };

    info!(
        request_id = request.request_id,
        connector = ?connector,
        command = request.command.name(),
        "处理本地 Connector 请求"
    );
    match request.command {
        Command::Status => {
            status_response(
                request.request_id,
                connector,
                chrome_seen,
                paths,
                hermes_client,
            )
            .await
        }
        Command::ConnectionAdd {
            name,
            base_url,
            model,
            token,
        } => {
            if *connector != ConnectorKind::Cli {
                return HostResponse::failure(
                    request.request_id,
                    ErrorCode::PermissionDenied,
                    "Chrome Connector 无权修改 Hermes Connection",
                    false,
                );
            }
            let operation =
                HermesConnection::direct(&name, &base_url, model).and_then(|connection| {
                    add_connection(
                        paths,
                        &KeychainCredentialStore,
                        connection,
                        token.as_bytes(),
                    )
                });
            if let Err(error) = operation {
                return HostResponse::failure(
                    request.request_id,
                    ErrorCode::InvalidMessage,
                    error.to_string(),
                    false,
                );
            }
            status_response(
                request.request_id,
                connector,
                chrome_seen,
                paths,
                hermes_client,
            )
            .await
        }
        Command::ConnectionRemove { identifier } => {
            if *connector != ConnectorKind::Cli {
                return HostResponse::failure(
                    request.request_id,
                    ErrorCode::PermissionDenied,
                    "Chrome Connector 无权修改 Hermes Connection",
                    false,
                );
            }
            if let Err(error) = remove_connection(paths, &KeychainCredentialStore, &identifier) {
                return HostResponse::failure(
                    request.request_id,
                    ErrorCode::InvalidMessage,
                    error.to_string(),
                    false,
                );
            }
            status_response(
                request.request_id,
                connector,
                chrome_seen,
                paths,
                hermes_client,
            )
            .await
        }
    }
}

fn decode_request(value: Value) -> Result<HostRequest, HostResponse> {
    let request_id = request_id_from_value(&value);
    let Some(protocol_version) = value.get("protocol_version").and_then(Value::as_u64) else {
        return Err(HostResponse::failure(
            &request_id,
            ErrorCode::InvalidMessage,
            "缺少 protocol_version",
            false,
        ));
    };
    if protocol_version != u64::from(PROTOCOL_VERSION) {
        return Err(HostResponse::failure(
            &request_id,
            ErrorCode::ProtocolVersionUnsupported,
            format!("不支持协议版本 {protocol_version}，当前版本为 {PROTOCOL_VERSION}"),
            false,
        ));
    }

    let command_type = value
        .get("command")
        .and_then(|command| command.get("type"))
        .and_then(Value::as_str);
    if !matches!(
        command_type,
        Some("status" | "connection_add" | "connection_remove")
    ) {
        return Err(HostResponse::failure(
            &request_id,
            ErrorCode::UnknownCommand,
            format!("未知命令 {}", command_type.unwrap_or("<missing>")),
            false,
        ));
    }

    serde_json::from_value(value).map_err(|error| {
        HostResponse::failure(
            request_id,
            ErrorCode::InvalidMessage,
            format!("请求结构无效: {error}"),
            false,
        )
    })
}

async fn status_response(
    request_id: String,
    connector: &ConnectorKind,
    chrome_seen: &AtomicBool,
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
) -> HostResponse {
    let targets = discover_hermes_targets(paths, hermes_client).await;
    HostResponse::success(
        request_id,
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
            targets,
        },
    )
}

async fn discover_hermes_targets(
    paths: &AgentFerryPaths,
    client: &HermesClient,
) -> Vec<HandoffTargetStatus> {
    let connections = match load_connections(&paths.hermes_connections) {
        Ok(connections) => connections,
        Err(error) => {
            warn!(error = %error, "无法读取 Hermes Connection 配置");
            return Vec::new();
        }
    };
    let store = KeychainCredentialStore;
    let mut targets = Vec::with_capacity(connections.connections.len());
    for connection in connections.connections {
        let diagnosis = match client.diagnose_with_store(&connection, &store).await {
            Ok(diagnosis) => diagnosis,
            Err(error) => {
                warn!(connection_id = connection.id, error = %error, "Hermes 诊断失败");
                targets.push(HandoffTargetStatus {
                    id: connection.id,
                    name: connection.name,
                    kind: HandoffTargetKind::RemoteHermes,
                    state: HandoffTargetState::ConnectionFailed,
                    capabilities: Vec::new(),
                });
                continue;
            }
        };
        targets.push(HandoffTargetStatus {
            id: diagnosis.id,
            name: diagnosis.name,
            kind: HandoffTargetKind::RemoteHermes,
            state: match diagnosis.state {
                DiagnosisState::Ready => HandoffTargetState::Ready,
                DiagnosisState::CredentialMissing => HandoffTargetState::CredentialMissing,
                DiagnosisState::AuthenticationFailed => HandoffTargetState::AuthenticationFailed,
                DiagnosisState::ConnectionFailed => HandoffTargetState::ConnectionFailed,
                DiagnosisState::Incompatible => HandoffTargetState::Incompatible,
            },
            capabilities: diagnosis.capabilities,
        });
    }
    targets
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
