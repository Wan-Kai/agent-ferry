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
    CredentialStore, DiagnosisState, HermesClient, HermesConnection, KeychainCredentialStore,
    add_connection, load_connections, remove_connection,
};
use agent_ferry_protocol::{
    Command, ConnectorKind, ErrorCode, HandoffEvent, HandoffTargetKind, HandoffTargetState,
    HandoffTargetStatus, HostRequest, HostResponse, IpcEnvelope, MAX_MESSAGE_BYTES,
    PROTOCOL_VERSION, ServiceState, SourceDocument, StatusResult,
};
use serde::Serialize;
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
    let request = match decode_request(envelope.request) {
        Ok(request) => request,
        Err(response) => {
            write_async_json(&mut stream, &response).await?;
            return Ok(());
        }
    };
    info!(
        request_id = request.request_id,
        connector = ?envelope.connector,
        command = request.command.name(),
        "处理本地 Connector 请求"
    );
    if let Command::Handoff {
        task_id,
        target_id,
        prompt,
        source,
    } = request.command
    {
        return stream_handoff(
            &mut stream,
            request.request_id,
            task_id,
            target_id,
            prompt,
            source,
            &envelope.connector,
            paths,
            hermes_client,
        )
        .await;
    }

    let response = dispatch_request(
        HostRequest {
            protocol_version: request.protocol_version,
            request_id: request.request_id,
            command: request.command,
        },
        &envelope.connector,
        chrome_seen,
        paths,
        hermes_client,
    )
    .await;
    write_async_json(&mut stream, &response).await
}

async fn dispatch_request(
    request: HostRequest,
    connector: &ConnectorKind,
    chrome_seen: &AtomicBool,
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
) -> HostResponse {
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
        Command::Handoff { .. } => HostResponse::failure(
            request.request_id,
            ErrorCode::Internal,
            "Handoff 未进入流式处理路径",
            false,
        ),
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
        Some("status" | "connection_add" | "connection_remove" | "handoff")
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn stream_handoff(
    stream: &mut UnixStream,
    request_id: String,
    task_id: String,
    target_id: String,
    prompt: String,
    source: SourceDocument,
    connector: &ConnectorKind,
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
) -> io::Result<()> {
    if *connector != ConnectorKind::ChromeNativeHost {
        return write_async_json(
            stream,
            &HostResponse::failure(
                request_id,
                ErrorCode::PermissionDenied,
                "只有浏览器 Connector 可以提交页面交接",
                false,
            ),
        )
        .await;
    }
    if task_id.trim().is_empty() || task_id.len() > 128 {
        return write_async_json(
            stream,
            &HostResponse::failure(
                request_id,
                ErrorCode::InvalidMessage,
                "task_id 不能为空且不得超过 128 字节",
                false,
            ),
        )
        .await;
    }
    let input = match compose_handoff_input(&prompt, &source) {
        Ok(input) => input,
        Err(message) => {
            return write_async_json(
                stream,
                &HostResponse::failure(request_id, ErrorCode::InvalidMessage, message, false),
            )
            .await;
        }
    };
    let connections = match load_connections(&paths.hermes_connections) {
        Ok(connections) => connections,
        Err(error) => {
            return write_async_json(
                stream,
                &HostResponse::failure(request_id, ErrorCode::Internal, error.to_string(), false),
            )
            .await;
        }
    };
    let Some(connection) = connections
        .connections
        .into_iter()
        .find(|candidate| candidate.id == target_id)
    else {
        return write_async_json(
            stream,
            &HostResponse::failure(
                request_id,
                ErrorCode::InvalidMessage,
                "目标 Hermes Connection 不存在",
                false,
            ),
        )
        .await;
    };
    let Some(token) = KeychainCredentialStore
        .get(&connection.credential_ref)
        .map_err(io::Error::other)?
    else {
        return write_async_json(
            stream,
            &HostResponse::failure(
                request_id,
                ErrorCode::PermissionDenied,
                "目标 Hermes 的 Keychain 凭据不存在",
                false,
            ),
        )
        .await;
    };
    let diagnosis = hermes_client
        .diagnose(&connection, &token)
        .await
        .map_err(io::Error::other)?;
    if diagnosis.state != DiagnosisState::Ready {
        return write_async_json(
            stream,
            &HostResponse::failure(
                request_id,
                ErrorCode::DaemonUnavailable,
                diagnosis.detail,
                true,
            ),
        )
        .await;
    }
    let use_sse = diagnosis
        .capabilities
        .iter()
        .any(|capability| capability == "run.events_sse");
    let mut updates = hermes_client.run(connection, token, input, use_sse);
    let mut sequence = 0_u64;
    while let Some(update) = updates.recv().await {
        info!(
            task_id,
            target_id,
            sequence,
            event = ?update.kind,
            "转发 Hermes Run 事件"
        );
        let event = HandoffEvent {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.clone(),
            task_id: task_id.clone(),
            sequence,
            event: update.kind,
            run_id: update.run_id,
            text: update.text,
        };
        if write_async_json(stream, &event).await.is_err() {
            // 弹窗关闭只中断本地观察；Hermes Run 已经由远端托管，不执行 stop。
            info!(task_id, "浏览器观察者已离开，远端 Run 继续执行");
            break;
        }
        sequence = sequence.saturating_add(1);
    }
    Ok(())
}

/// 形成唯一传给 Hermes 的 input，确保用户可见 prompt、来源元数据和正文不被拆散。
fn compose_handoff_input(prompt: &str, source: &SourceDocument) -> Result<String, String> {
    if prompt.trim().is_empty() || prompt.len() > 16 * 1024 {
        return Err("Prompt 不能为空且不得超过 16 KiB".to_owned());
    }
    if task_source_invalid(source) {
        return Err("页面正文为空、明显不完整或来源元数据无效，请等待页面加载后重试".to_owned());
    }
    Ok(format!(
        "{prompt}\n\n---\n来源 URL: {}\n标题: {}\n作者: {}\n发布日期: {}\n站点: {}\n提取器: {}\n字数: {}\n---\n\n{}",
        source.url,
        source.title,
        source.author.as_deref().unwrap_or("未知"),
        source.published.as_deref().unwrap_or("未知"),
        source.site.as_deref().unwrap_or("未知"),
        source.extractor,
        source.word_count,
        source.markdown
    ))
}

fn task_source_invalid(source: &SourceDocument) -> bool {
    let valid_url = url::Url::parse(&source.url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    !valid_url
        || source.title.trim().is_empty()
        || source.extractor != "defuddle"
        || source.markdown.trim().len() < 200
        || source.word_count < 40
        || source.markdown.len() > 800 * 1024
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
            capabilities: vec!["target.read".to_owned(), "handoff.submit".to_owned()],
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

async fn write_async_json<T: Serialize>(stream: &mut UnixStream, response: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(response).map_err(io::Error::other)?;
    let length = u32::try_from(payload.len()).map_err(io::Error::other)?;
    stream.write_u32_le(length).await?;
    stream.write_all(&payload).await
}

fn frame_error_code(error: &io::Error) -> ErrorCode {
    if error.kind() == io::ErrorKind::InvalidData {
        ErrorCode::MessageTooLarge
    } else {
        ErrorCode::InvalidMessage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_input_contains_visible_prompt_metadata_and_full_markdown() {
        let markdown = format!("# 正文\n\n{}", "这是用于验证完整交接的内容。".repeat(50));
        let source = SourceDocument {
            url: "https://example.com/article".to_owned(),
            title: "测试文章".to_owned(),
            author: Some("作者".to_owned()),
            published: Some("2026-07-16".to_owned()),
            site: Some("示例站点".to_owned()),
            extractor: "defuddle".to_owned(),
            markdown: markdown.clone(),
            word_count: 100,
        };
        let input = compose_handoff_input("  请分析\n", &source).expect("形成 input");
        assert!(input.starts_with("  请分析\n\n\n---"));
        assert!(input.contains("来源 URL: https://example.com/article"));
        assert!(input.ends_with(&markdown));
    }

    #[test]
    fn handoff_rejects_url_only_or_obviously_short_capture() {
        let source = SourceDocument {
            url: "https://example.com".to_owned(),
            title: "短页面".to_owned(),
            author: None,
            published: None,
            site: None,
            extractor: "defuddle".to_owned(),
            markdown: "只有链接".to_owned(),
            word_count: 2,
        };
        assert!(compose_handoff_input("分析", &source).is_err());
    }
}
