use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_HANDOFF_CHUNK_BYTES: usize = 192 * 1024;
pub const MAX_HANDOFF_CONTENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_HANDOFF_CHUNKS: u32 = 43;
pub const MAX_HERMES_RUN_INPUT_BYTES: usize = 512 * 1024;
pub const NATIVE_HOST_NAME: &str = "com.agentferry.host";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    ChromeNativeHost,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcEnvelope {
    pub auth_token: String,
    pub connector: ConnectorKind,
    pub request: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRequest {
    pub protocol_version: u16,
    pub request_id: String,
    pub command: Command,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionTransportConfig {
    #[default]
    Direct,
    SshTunnel {
        ssh_host: String,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Status,
    ConnectionAdd {
        name: String,
        base_url: String,
        model: Option<String>,
        #[serde(default)]
        transport: ConnectionTransportConfig,
        token: String,
    },
    ConnectionRemove {
        identifier: String,
    },
    WorkspaceAdd {
        name: String,
        path: String,
    },
    WorkspaceRemove {
        identifier: String,
    },
    HistoryList {
        state: Option<TaskHistoryState>,
        #[serde(default = "default_history_limit")]
        limit: u16,
    },
    HistoryGet {
        task_id: String,
    },
    HistoryDelete {
        task_id: String,
    },
    HermesRun {
        task_id: String,
        target_id: String,
        input: String,
    },
    Handoff {
        task_id: String,
        target_id: String,
        prompt: String,
        source: SourceDocument,
    },
    HandoffBegin {
        task_id: String,
        target_id: String,
        prompt: String,
        source: SourceMetadata,
        total_bytes: usize,
        total_chunks: u32,
        sha256: String,
    },
    HandoffChunk {
        task_id: String,
        index: u32,
        data: String,
    },
    HandoffEnd {
        task_id: String,
    },
}

impl std::fmt::Debug for Command {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Command 可能被上层错误链或诊断工具整体格式化；在协议层统一脱敏，避免未来新增日志时意外泄露 Bearer Token。
        match self {
            Self::Status => formatter.write_str("Status"),
            Self::ConnectionAdd {
                name,
                base_url,
                model,
                transport,
                token: _,
            } => formatter
                .debug_struct("ConnectionAdd")
                .field("name", name)
                .field("base_url", base_url)
                .field("model", model)
                .field("transport", transport)
                .field("token", &"[REDACTED]")
                .finish(),
            Self::ConnectionRemove { identifier } => formatter
                .debug_struct("ConnectionRemove")
                .field("identifier", identifier)
                .finish(),
            Self::WorkspaceAdd { name, path } => formatter
                .debug_struct("WorkspaceAdd")
                .field("name", name)
                .field("path", path)
                .finish(),
            Self::WorkspaceRemove { identifier } => formatter
                .debug_struct("WorkspaceRemove")
                .field("identifier", identifier)
                .finish(),
            Self::HistoryList { state, limit } => formatter
                .debug_struct("HistoryList")
                .field("state", state)
                .field("limit", limit)
                .finish(),
            Self::HistoryGet { task_id } => formatter
                .debug_struct("HistoryGet")
                .field("task_id", task_id)
                .finish(),
            Self::HistoryDelete { task_id } => formatter
                .debug_struct("HistoryDelete")
                .field("task_id", task_id)
                .finish(),
            Self::HermesRun {
                task_id,
                target_id,
                input: _,
            } => formatter
                .debug_struct("HermesRun")
                .field("task_id", task_id)
                .field("target_id", target_id)
                .field("input", &"[REDACTED]")
                .finish(),
            Self::Handoff {
                task_id,
                target_id,
                prompt: _,
                source,
            } => formatter
                .debug_struct("Handoff")
                .field("task_id", task_id)
                .field("target_id", target_id)
                .field("source_url", &source.url)
                .field("content", &"[REDACTED]")
                .finish(),
            Self::HandoffBegin {
                task_id,
                target_id,
                prompt: _,
                source,
                total_bytes,
                total_chunks,
                sha256: _,
            } => formatter
                .debug_struct("HandoffBegin")
                .field("task_id", task_id)
                .field("target_id", target_id)
                .field("source_url", &source.url)
                .field("total_bytes", total_bytes)
                .field("total_chunks", total_chunks)
                .field("content", &"[REDACTED]")
                .finish(),
            Self::HandoffChunk {
                task_id,
                index,
                data: _,
            } => formatter
                .debug_struct("HandoffChunk")
                .field("task_id", task_id)
                .field("index", index)
                .field("data", &"[REDACTED]")
                .finish(),
            Self::HandoffEnd { task_id } => formatter
                .debug_struct("HandoffEnd")
                .field("task_id", task_id)
                .finish(),
        }
    }
}

impl Command {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::ConnectionAdd { .. } => "connection_add",
            Self::ConnectionRemove { .. } => "connection_remove",
            Self::WorkspaceAdd { .. } => "workspace_add",
            Self::WorkspaceRemove { .. } => "workspace_remove",
            Self::HistoryList { .. } => "history_list",
            Self::HistoryGet { .. } => "history_get",
            Self::HistoryDelete { .. } => "history_delete",
            Self::HermesRun { .. } => "hermes_run",
            Self::Handoff { .. } => "handoff",
            Self::HandoffBegin { .. } => "handoff_begin",
            Self::HandoffChunk { .. } => "handoff_chunk",
            Self::HandoffEnd { .. } => "handoff_end",
        }
    }
}

const fn default_history_limit() -> u16 {
    50
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDocument {
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub published: Option<String>,
    pub site: Option<String>,
    pub extractor: String,
    pub markdown: String,
    pub word_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub published: Option<String>,
    pub site: Option<String>,
    pub extractor: String,
    pub word_count: usize,
}

impl SourceMetadata {
    #[must_use]
    pub fn with_markdown(self, markdown: String) -> SourceDocument {
        SourceDocument {
            url: self.url,
            title: self.title,
            author: self.author,
            published: self.published,
            site: self.site,
            extractor: self.extractor,
            markdown,
            word_count: self.word_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffTransferPhase {
    Begin,
    Chunk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTransferAck {
    pub protocol_version: u16,
    pub request_id: String,
    pub task_id: String,
    pub phase: HandoffTransferPhase,
    pub next_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffEventKind {
    Submitted,
    Running,
    OutputDelta,
    ToolStarted,
    ToolCompleted,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffEvent {
    pub protocol_version: u16,
    pub request_id: String,
    pub task_id: String,
    pub sequence: u64,
    pub event: HandoffEventKind,
    pub run_id: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistoryState {
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TaskHistoryState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryEvent {
    pub sequence: u64,
    pub event: HandoffEventKind,
    pub timestamp_ms: u64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistorySummary {
    pub task_id: String,
    pub title: String,
    pub url: String,
    pub site: Option<String>,
    pub extractor: String,
    pub target_id: String,
    pub target_name: String,
    pub workspace_name: Option<String>,
    pub workspace_path: Option<String>,
    pub state: TaskHistoryState,
    pub stage: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryRecord {
    pub summary: TaskHistorySummary,
    pub prompt: String,
    pub output: String,
    pub output_truncated: bool,
    pub error: Option<String>,
    pub run_id: Option<String>,
    pub events: Vec<TaskHistoryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryListResult {
    pub tasks: Vec<TaskHistorySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryGetResult {
    pub task: Option<TaskHistoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryDeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryResponse<T> {
    pub protocol_version: u16,
    pub request_id: String,
    pub result: T,
}

impl<T> TaskHistoryResponse<T> {
    #[must_use]
    pub fn new(request_id: impl Into<String>, result: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResponse {
    pub protocol_version: u16,
    pub request_id: String,
    #[serde(flatten)]
    pub outcome: ResponseOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Ready,
    NotDetected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffTargetKind {
    RemoteHermes,
    LocalOpenCode,
    LocalClaudeCode,
    LocalCodexCli,
    LocalCodexApp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalWorkspaceStatus {
    pub id: String,
    pub name: String,
    pub path: String,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffTargetState {
    Ready,
    CredentialMissing,
    AuthenticationFailed,
    ConnectionFailed,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTargetStatus {
    pub id: String,
    pub name: String,
    pub kind: HandoffTargetKind,
    pub state: HandoffTargetState,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResult {
    pub core_version: String,
    pub daemon: ServiceState,
    pub native_host: ServiceState,
    pub chrome_extension: ServiceState,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub targets: Box<[HandoffTargetStatus]>,
    #[serde(default)]
    pub workspaces: Box<[LocalWorkspaceStatus]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseOutcome {
    Success { result: StatusResult },
    Failure { error: ProtocolError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AuthenticationFailed,
    PermissionDenied,
    DaemonUnavailable,
    InvalidMessage,
    MessageTooLarge,
    ProtocolVersionUnsupported,
    UnknownCommand,
    Internal,
}

impl HostResponse {
    #[must_use]
    pub fn success(request_id: impl Into<String>, result: StatusResult) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            outcome: ResponseOutcome::Success { result },
        }
    }

    #[must_use]
    pub fn failure(
        request_id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            outcome: ResponseOutcome::Failure {
                error: ProtocolError {
                    code,
                    message: message.into(),
                    recoverable,
                },
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("消息流已经结束")]
    EndOfStream,
    #[error("消息长度 {actual} 超过上限 {maximum}")]
    MessageTooLarge { actual: usize, maximum: usize },
    #[error("消息读写失败: {0}")]
    Io(#[from] io::Error),
    #[error("消息不是有效 JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Chrome Native Messaging 和本地 IPC 共用同一种小端长度前缀，避免桥接层
/// 为两种 framing 维护不同实现。正文上限在分配缓冲区之前检查，防止不可信
/// 页面通过伪造长度触发大内存分配。
///
/// # Errors
///
/// 输入结束、底层读取失败或消息超过长度上限时返回错误。
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut length_bytes = [0_u8; 4];
    match reader.read_exact(&mut length_bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::EndOfStream);
        }
        Err(error) => return Err(FrameError::Io(error)),
    }

    let length = u32::from_le_bytes(length_bytes) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(FrameError::MessageTooLarge {
            actual: length,
            maximum: MAX_MESSAGE_BYTES,
        });
    }

    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// 写入 Chrome Native Messaging 兼容的长度前缀消息。
///
/// # Errors
///
/// 正文超过长度上限或底层写入失败时返回错误。
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::MessageTooLarge {
            actual: payload.len(),
            maximum: MAX_MESSAGE_BYTES,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| FrameError::MessageTooLarge {
        actual: payload.len(),
        maximum: MAX_MESSAGE_BYTES,
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// 读取并反序列化一条带长度前缀的 JSON 消息。
///
/// # Errors
///
/// framing 读取失败或正文不是目标 JSON 结构时返回错误。
pub fn read_json_frame<R: Read, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> Result<T, FrameError> {
    let payload = read_frame(reader)?;
    Ok(serde_json::from_slice(&payload)?)
}

/// 序列化并写入一条带长度前缀的 JSON 消息。
///
/// # Errors
///
/// JSON 序列化失败、消息超过长度上限或底层写入失败时返回错误。
pub fn write_json_frame<W: Write, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value)?;
    write_frame(writer, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_frame_round_trip() {
        let request = HostRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: "request-1".to_owned(),
            command: Command::Status,
        };
        let mut bytes = Vec::new();
        write_json_frame(&mut bytes, &request).expect("写入 frame");

        let decoded: HostRequest = read_json_frame(&mut bytes.as_slice()).expect("读取 frame");
        assert_eq!(decoded, request);
    }

    #[test]
    fn command_debug_never_exposes_connection_token() {
        let command = Command::ConnectionAdd {
            name: "remote".to_owned(),
            base_url: "https://hermes.example".to_owned(),
            model: None,
            transport: ConnectionTransportConfig::Direct,
            token: "must-stay-secret".to_owned(),
        };

        let rendered = format!("{command:?}");
        assert!(!rendered.contains("must-stay-secret"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn legacy_connection_add_defaults_to_direct_transport() {
        let request: HostRequest = serde_json::from_value(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "legacy",
            "command": {
                "type": "connection_add",
                "name": "remote",
                "base_url": "http://127.0.0.1:8642",
                "model": null,
                "token": "secret"
            }
        }))
        .expect("解析旧版 ConnectionAdd");
        assert!(matches!(
            request.command,
            Command::ConnectionAdd {
                transport: ConnectionTransportConfig::Direct,
                ..
            }
        ));
    }

    #[test]
    fn chunk_debug_never_exposes_page_content() {
        let command = Command::HandoffChunk {
            task_id: "task-1".to_owned(),
            index: 2,
            data: "must-stay-private-page-content".to_owned(),
        };
        let rendered = format!("{command:?}");
        assert!(!rendered.contains("must-stay-private-page-content"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn hermes_run_debug_never_exposes_input() {
        let command = Command::HermesRun {
            task_id: "task-1".to_owned(),
            target_id: "remote-1".to_owned(),
            input: "must-stay-private-run-input".to_owned(),
        };
        let rendered = format!("{command:?}");
        assert!(!rendered.contains("must-stay-private-run-input"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn oversized_frame_is_rejected_before_payload_read() {
        let oversized = u32::try_from(MAX_MESSAGE_BYTES).expect("上限应能表示为 u32") + 1;
        let mut bytes = oversized.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"ignored");
        let error = read_frame(&mut bytes.as_slice()).expect_err("应拒绝超大消息");
        assert!(matches!(error, FrameError::MessageTooLarge { .. }));
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let mut bytes = 10_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"short");
        let error = read_frame(&mut bytes.as_slice()).expect_err("应拒绝截断消息");
        assert!(matches!(error, FrameError::Io(_)));
    }

    #[test]
    fn codex_target_kinds_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&HandoffTargetKind::LocalCodexCli)
                .expect("序列化 Codex CLI kind"),
            "\"local_codex_cli\""
        );
        assert_eq!(
            serde_json::to_string(&HandoffTargetKind::LocalCodexApp)
                .expect("序列化 Codex App kind"),
            "\"local_codex_app\""
        );
    }

    #[test]
    fn history_commands_have_stable_wire_names() {
        let request = HostRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: "history-1".to_owned(),
            command: Command::HistoryList {
                state: Some(TaskHistoryState::Running),
                limit: 50,
            },
        };

        let value = serde_json::to_value(request).expect("序列化历史查询");
        assert_eq!(value["command"]["type"], "history_list");
        assert_eq!(value["command"]["state"], "running");
        assert_eq!(value["command"]["limit"], 50);
    }
}
