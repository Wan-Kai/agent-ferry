use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use agent_ferry_core::{AgentFerryPaths, load_or_create_connector_token};
use agent_ferry_hermes::{
    CredentialStore, DiagnosisState, HermesClient, HermesConnection, KeychainCredentialStore,
    add_connection, load_connections, remove_connection,
};
use agent_ferry_protocol::{
    Command, ConnectorKind, ErrorCode, HandoffEvent, HandoffTargetKind, HandoffTargetState,
    HandoffTargetStatus, HandoffTransferAck, HandoffTransferPhase, HostRequest, HostResponse,
    IpcEnvelope, MAX_HANDOFF_CHUNK_BYTES, MAX_HANDOFF_CHUNKS, MAX_HANDOFF_CONTENT_BYTES,
    MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ServiceState, SourceDocument, SourceMetadata,
    StatusResult,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug)]
pub struct Daemon {
    paths: AgentFerryPaths,
    listener: UnixListener,
    auth_token: Arc<str>,
    chrome_seen: Arc<AtomicBool>,
    hermes_client: Arc<HermesClient>,
    handoff_assemblies: Arc<Mutex<HashMap<String, HandoffAssembly>>>,
}

const MAX_ACTIVE_HANDOFF_ASSEMBLIES: usize = 8;
const HANDOFF_ASSEMBLY_TTL: Duration = Duration::from_secs(5 * 60);

struct HandoffAssembly {
    target_id: String,
    prompt: String,
    source: SourceMetadata,
    total_bytes: usize,
    total_chunks: u32,
    sha256: String,
    next_index: u32,
    bytes: Vec<u8>,
    updated_at: Instant,
}

impl std::fmt::Debug for HandoffAssembly {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HandoffAssembly")
            .field("target_id", &self.target_id)
            .field("source_url", &self.source.url)
            .field("total_bytes", &self.total_bytes)
            .field("total_chunks", &self.total_chunks)
            .field("next_index", &self.next_index)
            .field("content", &"[REDACTED]")
            .finish_non_exhaustive()
    }
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
            handoff_assemblies: Arc::new(Mutex::new(HashMap::new())),
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
                    let handoff_assemblies = Arc::clone(&self.handoff_assemblies);
                    tokio::spawn(async move {
                        if let Err(error) = handle_connection(
                            stream,
                            &token,
                            &chrome_seen,
                            &paths,
                            &hermes_client,
                            &handoff_assemblies,
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

#[allow(clippy::too_many_lines)]
async fn handle_connection(
    mut stream: UnixStream,
    expected_token: &str,
    chrome_seen: &AtomicBool,
    paths: &AgentFerryPaths,
    hermes_client: &HermesClient,
    handoff_assemblies: &Mutex<HashMap<String, HandoffAssembly>>,
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
    let request_id = request.request_id;
    match request.command {
        Command::Handoff {
            task_id,
            target_id,
            prompt,
            source,
        } => {
            stream_handoff(
                &mut stream,
                request_id,
                task_id,
                target_id,
                prompt,
                source,
                &envelope.connector,
                paths,
                hermes_client,
            )
            .await
        }
        Command::HandoffBegin {
            task_id,
            target_id,
            prompt,
            source,
            total_bytes,
            total_chunks,
            sha256,
        } => {
            let result = begin_handoff_transfer(
                &request_id,
                task_id,
                target_id,
                prompt,
                source,
                total_bytes,
                total_chunks,
                sha256,
                &envelope.connector,
                handoff_assemblies,
            )
            .await;
            write_transfer_result(&mut stream, result).await
        }
        Command::HandoffChunk {
            task_id,
            index,
            data,
        } => {
            let result = append_handoff_chunk(
                &request_id,
                task_id,
                index,
                data,
                &envelope.connector,
                handoff_assemblies,
            )
            .await;
            write_transfer_result(&mut stream, result).await
        }
        Command::HandoffEnd { task_id } => {
            match finish_handoff_transfer(
                &request_id,
                task_id,
                &envelope.connector,
                handoff_assemblies,
            )
            .await
            {
                Ok(handoff) => {
                    stream_handoff(
                        &mut stream,
                        request_id,
                        handoff.task_id,
                        handoff.target_id,
                        handoff.prompt,
                        handoff.source,
                        &envelope.connector,
                        paths,
                        hermes_client,
                    )
                    .await
                }
                Err(response) => write_async_json(&mut stream, &response).await,
            }
        }
        command => {
            let response = dispatch_request(
                HostRequest {
                    protocol_version: request.protocol_version,
                    request_id,
                    command,
                },
                &envelope.connector,
                chrome_seen,
                paths,
                hermes_client,
            )
            .await;
            write_async_json(&mut stream, &response).await
        }
    }
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
        Command::Handoff { .. }
        | Command::HandoffBegin { .. }
        | Command::HandoffChunk { .. }
        | Command::HandoffEnd { .. } => HostResponse::failure(
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
        Some(
            "status"
                | "connection_add"
                | "connection_remove"
                | "handoff"
                | "handoff_begin"
                | "handoff_chunk"
                | "handoff_end"
        )
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

struct PreparedHandoff {
    task_id: String,
    target_id: String,
    prompt: String,
    source: SourceDocument,
}

#[allow(clippy::too_many_arguments)]
async fn begin_handoff_transfer(
    request_id: &str,
    task_id: String,
    target_id: String,
    prompt: String,
    source: SourceMetadata,
    total_bytes: usize,
    total_chunks: u32,
    sha256: String,
    connector: &ConnectorKind,
    assemblies: &Mutex<HashMap<String, HandoffAssembly>>,
) -> Result<HandoffTransferAck, HostResponse> {
    validate_transfer_connector(request_id, connector)?;
    validate_task_id(request_id, &task_id)?;
    if !safe_identifier(&target_id) {
        return Err(transfer_failure(request_id, "target_id 无效"));
    }
    if prompt.trim().is_empty() || prompt.len() > 16 * 1024 {
        return Err(transfer_failure(
            request_id,
            "Prompt 不能为空且不得超过 16 KiB",
        ));
    }
    if source_metadata_invalid(&source) {
        return Err(transfer_failure(request_id, "页面来源元数据无效"));
    }
    let minimum_bytes = minimum_markdown_length(&source.extractor);
    if total_bytes < minimum_bytes {
        return Err(transfer_failure(
            request_id,
            format!("正文不得少于 {minimum_bytes} 字节"),
        ));
    }
    if total_bytes > MAX_HANDOFF_CONTENT_BYTES {
        return Err(HostResponse::failure(
            request_id,
            ErrorCode::MessageTooLarge,
            format!(
                "正文大小必须在 {minimum_bytes} 字节到 {} MiB 之间",
                MAX_HANDOFF_CONTENT_BYTES / 1024 / 1024
            ),
            false,
        ));
    }
    if total_chunks == 0 || total_chunks > MAX_HANDOFF_CHUNKS {
        return Err(transfer_failure(request_id, "正文分块数量超出协议上限"));
    }
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(transfer_failure(
            request_id,
            "sha256 必须是 64 位十六进制字符串",
        ));
    }

    let mut active = assemblies.lock().await;
    active.retain(|_, assembly| assembly.updated_at.elapsed() <= HANDOFF_ASSEMBLY_TTL);
    if active.contains_key(&task_id) {
        return Err(transfer_failure(request_id, "task_id 已存在未完成的传输"));
    }
    if active.len() >= MAX_ACTIVE_HANDOFF_ASSEMBLIES {
        // 无法从独立 IPC 连接判断 begin 是否已被客户端放弃，因此绝不能静默淘汰已 ACK 的任务。
        // 中断项由 TTL 回收；满额期间显式拒绝新任务，让发送方稍后重试。
        return Err(HostResponse::failure(
            request_id,
            ErrorCode::MessageTooLarge,
            "并发正文传输数量已达上限，请稍后重试",
            true,
        ));
    }
    active.insert(
        task_id.clone(),
        HandoffAssembly {
            target_id,
            prompt,
            source,
            total_bytes,
            total_chunks,
            sha256: sha256.to_ascii_lowercase(),
            next_index: 0,
            // begin 可能永远收不到 chunk；延迟分配避免中断传输仅凭声明就长期占用 8 MiB。
            bytes: Vec::new(),
            updated_at: Instant::now(),
        },
    );
    Ok(HandoffTransferAck {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.to_owned(),
        task_id,
        phase: HandoffTransferPhase::Begin,
        next_index: 0,
    })
}

async fn append_handoff_chunk(
    request_id: &str,
    task_id: String,
    index: u32,
    data: String,
    connector: &ConnectorKind,
    assemblies: &Mutex<HashMap<String, HandoffAssembly>>,
) -> Result<HandoffTransferAck, HostResponse> {
    validate_transfer_connector(request_id, connector)?;
    validate_task_id(request_id, &task_id)?;
    let mut active = assemblies.lock().await;
    let Some(mut assembly) = active.remove(&task_id) else {
        return Err(transfer_failure(request_id, "未找到对应的 handoff_begin"));
    };
    let chunk_bytes = data.as_bytes();
    if chunk_bytes.is_empty() || chunk_bytes.len() > MAX_HANDOFF_CHUNK_BYTES {
        // 任意非法 chunk 都终止整个 assembly，禁止发送方在收到失败后继续拼出一个可提交的 Run。
        return Err(HostResponse::failure(
            request_id,
            ErrorCode::MessageTooLarge,
            "chunk 不能为空且不得超过 192 KiB",
            false,
        ));
    }
    if assembly.updated_at.elapsed() > HANDOFF_ASSEMBLY_TTL {
        return Err(transfer_failure(request_id, "正文传输已过期，请重新发送"));
    }
    if index != assembly.next_index {
        return Err(transfer_failure(
            request_id,
            format!("chunk 顺序错误：期望 {}，收到 {index}", assembly.next_index),
        ));
    }
    if index >= assembly.total_chunks
        || assembly.bytes.len().saturating_add(chunk_bytes.len()) > assembly.total_bytes
    {
        return Err(transfer_failure(request_id, "chunk 超出声明的正文边界"));
    }
    assembly.bytes.extend_from_slice(chunk_bytes);
    assembly.next_index += 1;
    assembly.updated_at = Instant::now();
    let next_index = assembly.next_index;
    active.insert(task_id.clone(), assembly);
    Ok(HandoffTransferAck {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.to_owned(),
        task_id,
        phase: HandoffTransferPhase::Chunk,
        next_index,
    })
}

async fn finish_handoff_transfer(
    request_id: &str,
    task_id: String,
    connector: &ConnectorKind,
    assemblies: &Mutex<HashMap<String, HandoffAssembly>>,
) -> Result<PreparedHandoff, HostResponse> {
    validate_transfer_connector(request_id, connector)?;
    validate_task_id(request_id, &task_id)?;
    let Some(assembly) = assemblies.lock().await.remove(&task_id) else {
        return Err(transfer_failure(request_id, "未找到对应的正文传输"));
    };
    if assembly.next_index != assembly.total_chunks {
        return Err(transfer_failure(
            request_id,
            format!(
                "正文分块不完整：期望 {} 块，只收到 {} 块",
                assembly.total_chunks, assembly.next_index
            ),
        ));
    }
    if assembly.bytes.len() != assembly.total_bytes {
        return Err(transfer_failure(
            request_id,
            format!(
                "正文大小不一致：期望 {} 字节，收到 {} 字节",
                assembly.total_bytes,
                assembly.bytes.len()
            ),
        ));
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&assembly.bytes));
    if actual_sha256 != assembly.sha256 {
        return Err(transfer_failure(request_id, "正文 sha256 完整性校验失败"));
    }
    let markdown = String::from_utf8(assembly.bytes)
        .map_err(|_| transfer_failure(request_id, "正文不是有效 UTF-8"))?;
    Ok(PreparedHandoff {
        task_id,
        target_id: assembly.target_id,
        prompt: assembly.prompt,
        source: assembly.source.with_markdown(markdown),
    })
}

fn validate_transfer_connector(
    request_id: &str,
    connector: &ConnectorKind,
) -> Result<(), HostResponse> {
    if *connector == ConnectorKind::ChromeNativeHost {
        Ok(())
    } else {
        Err(HostResponse::failure(
            request_id,
            ErrorCode::PermissionDenied,
            "只有浏览器 Connector 可以传输页面正文",
            false,
        ))
    }
}

fn validate_task_id(request_id: &str, task_id: &str) -> Result<(), HostResponse> {
    if safe_identifier(task_id) {
        Ok(())
    } else {
        Err(transfer_failure(
            request_id,
            "task_id 只能包含字母、数字、连字符或下划线，且不得超过 128 字节",
        ))
    }
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn source_metadata_invalid(source: &SourceMetadata) -> bool {
    !source_url_valid_for_extractor(&source.url, &source.extractor)
        || source.url.len() > 8 * 1024
        || source.title.trim().is_empty()
        || source.title.len() > 1024
        || !matches!(
            source.extractor.as_str(),
            "defuddle" | "x-thread" | "arxiv-html" | "arxiv-pdf"
        )
        || source.word_count < minimum_word_count(&source.extractor)
        || source.word_count > 100_000_000
        || [&source.author, &source.published, &source.site]
            .into_iter()
            .flatten()
            .any(|value| value.len() > 1024)
}

fn transfer_failure(request_id: &str, message: impl Into<String>) -> HostResponse {
    HostResponse::failure(request_id, ErrorCode::InvalidMessage, message, false)
}

async fn write_transfer_result(
    stream: &mut UnixStream,
    result: Result<HandoffTransferAck, HostResponse>,
) -> io::Result<()> {
    match result {
        Ok(ack) => write_async_json(stream, &ack).await,
        Err(response) => write_async_json(stream, &response).await,
    }
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
    !source_url_valid_for_extractor(&source.url, &source.extractor)
        || source.title.trim().is_empty()
        || !matches!(
            source.extractor.as_str(),
            "defuddle" | "x-thread" | "arxiv-html" | "arxiv-pdf"
        )
        || source.markdown.trim().len() < minimum_markdown_length(&source.extractor)
        || source.word_count < minimum_word_count(&source.extractor)
        || source.markdown.len() > MAX_HANDOFF_CONTENT_BYTES
}

fn minimum_word_count(extractor: &str) -> usize {
    // 单条 X 帖子天然可能很短；结构化提取器已要求永久链接命中当前 status，不能套用长文章阈值。
    if extractor == "x-thread" { 1 } else { 40 }
}

fn minimum_markdown_length(extractor: &str) -> usize {
    if extractor == "x-thread" { 40 } else { 200 }
}

fn source_url_valid_for_extractor(value: &str, extractor: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if matches!(extractor, "arxiv-html" | "arxiv-pdf") {
        let valid_host = url.host_str().is_some_and(|host| {
            matches!(
                host.to_ascii_lowercase().as_str(),
                "arxiv.org" | "www.arxiv.org"
            )
        });
        let prefix = if extractor == "arxiv-html" {
            "/html/"
        } else {
            "/pdf/"
        };
        let identifier = url
            .path()
            .strip_prefix(prefix)
            .unwrap_or_default()
            .strip_suffix('/')
            .unwrap_or_else(|| url.path().strip_prefix(prefix).unwrap_or_default());
        let identifier = if extractor == "arxiv-pdf" {
            identifier
                .rsplit_once('.')
                .map_or(identifier, |(stem, suffix)| {
                    if suffix.eq_ignore_ascii_case("pdf") {
                        stem
                    } else {
                        identifier
                    }
                })
        } else {
            identifier
        };
        return valid_host && valid_arxiv_identifier(identifier);
    }
    if extractor != "x-thread" {
        return extractor == "defuddle";
    }
    let valid_host = url.host_str().is_some_and(|host| {
        matches!(
            host.to_ascii_lowercase().as_str(),
            "x.com" | "www.x.com" | "twitter.com" | "www.twitter.com"
        )
    });
    let mut segments = url.path_segments().into_iter().flatten();
    let user = segments.next().unwrap_or_default();
    let status = segments.next().unwrap_or_default();
    let id = segments.next().unwrap_or_default();
    valid_host
        && !user.is_empty()
        && status == "status"
        && !id.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_arxiv_identifier(identifier: &str) -> bool {
    fn versioned_digits(value: &str, base_lengths: &[usize]) -> bool {
        let (base, version) = value.find(['v', 'V']).map_or((value, None), |index| {
            let (base, version) = value.split_at(index);
            (base, Some(&version[1..]))
        });
        base_lengths.contains(&base.len())
            && base.bytes().all(|byte| byte.is_ascii_digit())
            && version.is_none_or(|version| {
                !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
            })
    }

    if let Some((year_month, sequence)) = identifier.split_once('.') {
        if year_month.len() == 4
            && year_month.bytes().all(|byte| byte.is_ascii_digit())
            && versioned_digits(sequence, &[4, 5])
        {
            return true;
        }
    }
    let Some((archive, number)) = identifier.split_once('/') else {
        return false;
    };
    let valid_archive = archive.split_once('.').map_or_else(
        || {
            archive
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
        },
        |(base, suffix)| {
            !base.is_empty()
                && base
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
                && suffix.len() == 2
                && suffix.bytes().all(|byte| byte.is_ascii_alphabetic())
        },
    );
    !archive.is_empty() && valid_archive && versioned_digits(number, &[7])
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
            capabilities: vec![
                "target.read".to_owned(),
                "handoff.submit".to_owned(),
                "handoff.chunked".to_owned(),
            ],
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

    #[test]
    fn handoff_accepts_a_short_structured_x_post() {
        let source = SourceDocument {
            url: "https://x.com/agentferry/status/100".to_owned(),
            title: "X 线程：@agentferry".to_owned(),
            author: Some("Agent Ferry @agentferry".to_owned()),
            published: Some("2026-07-16T01:02:03.000Z".to_owned()),
            site: Some("X (Twitter)".to_owned()),
            extractor: "x-thread".to_owned(),
            markdown: "# X 线程：@agentferry\n\n### 主帖\n\n- 链接：https://x.com/agentferry/status/100\n\n好的。".to_owned(),
            word_count: 5,
        };
        assert!(compose_handoff_input("分析", &source).is_ok());
    }

    #[tokio::test]
    async fn chunked_begin_accepts_short_x_only_for_a_real_status_url() {
        let source = SourceMetadata {
            url: "https://x.com/agentferry/status/100".to_owned(),
            title: "X 对话：@agentferry".to_owned(),
            author: Some("Agent Ferry @agentferry".to_owned()),
            published: Some("2026-07-16T01:02:03.000Z".to_owned()),
            site: Some("X (Twitter)".to_owned()),
            extractor: "x-thread".to_owned(),
            word_count: 5,
        };
        let assemblies = Mutex::new(HashMap::new());
        let result = begin_handoff_transfer(
            "request",
            "task-short-x".to_owned(),
            "target".to_owned(),
            "分析".to_owned(),
            source.clone(),
            80,
            1,
            "0".repeat(64),
            &ConnectorKind::ChromeNativeHost,
            &assemblies,
        )
        .await;
        assert!(result.is_ok());

        let mut forged = source;
        forged.url = "https://example.com/article".to_owned();
        assert!(source_metadata_invalid(&forged));
        assert!(source_url_valid_for_extractor(
            "https://twitter.com/agentferry/status/100/photo/1",
            "x-thread"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://x.com/agentferry/status/100evil",
            "x-thread"
        ));
    }

    #[test]
    fn arxiv_html_extractor_is_bound_to_a_valid_paper_route() {
        assert!(source_url_valid_for_extractor(
            "https://arxiv.org/html/2402.08954v2",
            "arxiv-html"
        ));
        assert!(source_url_valid_for_extractor(
            "https://arxiv.org/html/2402.08954V2",
            "arxiv-html"
        ));
        assert!(source_url_valid_for_extractor(
            "https://www.arxiv.org/html/hep-th/9901001v1",
            "arxiv-html"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/abs/2402.08954",
            "arxiv-html"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://example.com/html/2402.08954",
            "arxiv-html"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/html/2402.08954evil",
            "arxiv-html"
        ));
    }

    #[test]
    fn arxiv_pdf_extractor_is_bound_to_a_valid_paper_route() {
        assert!(source_url_valid_for_extractor(
            "https://arxiv.org/pdf/2402.08954v2.pdf",
            "arxiv-pdf"
        ));
        assert!(source_url_valid_for_extractor(
            "https://arxiv.org/pdf/2402.08954v2.PdF",
            "arxiv-pdf"
        ));
        assert!(source_url_valid_for_extractor(
            "https://www.arxiv.org/pdf/hep-th/9901001V1?download=1",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/html/2402.08954",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://example.com/pdf/2402.08954.pdf",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/pdf/2402.08954evil.pdf",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/pdf/2402.089%35%34.pdf",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/pdf/hep..th/9901001.pdf",
            "arxiv-pdf"
        ));
        assert!(!source_url_valid_for_extractor(
            "https://arxiv.org/pdf/2402.08954.pdf//",
            "arxiv-pdf"
        ));
    }
}
