use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Status,
    ConnectionAdd {
        name: String,
        base_url: String,
        model: Option<String>,
        token: String,
    },
    ConnectionRemove {
        identifier: String,
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
                token: _,
            } => formatter
                .debug_struct("ConnectionAdd")
                .field("name", name)
                .field("base_url", base_url)
                .field("model", model)
                .field("token", &"[REDACTED]")
                .finish(),
            Self::ConnectionRemove { identifier } => formatter
                .debug_struct("ConnectionRemove")
                .field("identifier", identifier)
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
    pub targets: Vec<HandoffTargetStatus>,
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
            token: "must-stay-secret".to_owned(),
        };

        let rendered = format!("{command:?}");
        assert!(!rendered.contains("must-stay-secret"));
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
}
