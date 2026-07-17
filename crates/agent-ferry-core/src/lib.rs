use std::env;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use agent_ferry_protocol::{
    ConnectorKind, HostResponse, IpcEnvelope, NATIVE_HOST_NAME, read_json_frame, write_json_frame,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub mod workspace;

pub const HOME_ENV: &str = "AGENT_FERRY_HOME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFerryPaths {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub hermes_connections: PathBuf,
    pub claude_binding: PathBuf,
    pub opencode_binding: PathBuf,
    pub workspaces: PathBuf,
    pub run_dir: PathBuf,
    pub socket: PathBuf,
    pub connector_token: PathBuf,
    #[cfg(debug_assertions)]
    pub development_credentials: PathBuf,
    pub native_host_manifest: PathBuf,
}

impl AgentFerryPaths {
    /// 发现当前用户的 Agent Ferry 与 Chrome 配置路径。
    ///
    /// # Errors
    ///
    /// 无法确定用户目录时返回错误。
    pub fn discover() -> Result<Self, CoreError> {
        let root = if let Some(root) = env::var_os(HOME_ENV) {
            PathBuf::from(root)
        } else {
            let home = env::var_os("HOME").ok_or(CoreError::HomeDirectoryUnavailable)?;
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Agent Ferry")
        };
        let mut paths = Self::from_root(root);
        let home = env::var_os("HOME").ok_or(CoreError::HomeDirectoryUnavailable)?;
        paths.native_host_manifest = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Google")
            .join("Chrome")
            .join("NativeMessagingHosts")
            .join(format!("{NATIVE_HOST_NAME}.json"));
        Ok(paths)
    }

    #[must_use]
    pub fn from_root(root: PathBuf) -> Self {
        let config_dir = root.join("config");
        let run_dir = root.join("run");
        let native_host_manifest = root
            .join("native-messaging")
            .join(format!("{NATIVE_HOST_NAME}.json"));
        Self {
            socket: run_dir.join("agentferryd.sock"),
            connector_token: run_dir.join("connector.token"),
            #[cfg(debug_assertions)]
            development_credentials: root.join("dev").join("hermes-credentials.json"),
            hermes_connections: config_dir.join("hermes-connections.json"),
            claude_binding: config_dir.join("claude-code.json"),
            opencode_binding: config_dir.join("opencode.json"),
            workspaces: config_dir.join("workspaces.json"),
            config_dir,
            run_dir,
            native_host_manifest,
            root,
        }
    }

    /// 创建仅限当前用户访问的运行目录。
    ///
    /// # Errors
    ///
    /// 目录创建或权限设置失败时返回错误。
    pub fn ensure_private_runtime(&self) -> Result<(), CoreError> {
        fs::create_dir_all(&self.run_dir)?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&self.run_dir, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    /// 创建仅限当前用户访问的持久配置目录。
    ///
    /// # Errors
    ///
    /// 目录创建或权限设置失败时返回错误。
    pub fn ensure_private_config(&self) -> Result<(), CoreError> {
        fs::create_dir_all(&self.config_dir)?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&self.config_dir, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeHostManifest {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    #[serde(rename = "type")]
    pub transport_type: String,
    pub allowed_origins: Vec<String>,
}

impl NativeHostManifest {
    #[must_use]
    pub fn new(host_path: PathBuf, extension_id: &str) -> Self {
        Self {
            name: NATIVE_HOST_NAME.to_owned(),
            description: "Agent Ferry Chrome Native Messaging Bridge".to_owned(),
            path: host_path,
            transport_type: "stdio".to_owned(),
            allowed_origins: vec![format!("chrome-extension://{extension_id}/")],
        }
    }

    #[must_use]
    pub fn accepts_only_extension(&self, extension_id: &str) -> bool {
        self.allowed_origins == [format!("chrome-extension://{extension_id}/")]
    }
}

/// Connector token 不是用来抵抗已经完全控制当前 macOS 账号的恶意程序，
/// 而是防止误连接和未经过 Native Host/CLI 的普通本地客户端直接调用 daemon。
/// 正式分发还会依赖 Native Messaging allowlist 和代码签名边界。
///
/// # Errors
///
/// 私有运行目录、token 创建或读取失败时返回错误。
pub fn load_or_create_connector_token(paths: &AgentFerryPaths) -> Result<String, CoreError> {
    paths.ensure_private_runtime()?;
    if paths.connector_token.exists() {
        return load_connector_token(paths);
    }

    let token = Uuid::new_v4().simple().to_string();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    match options.open(&paths.connector_token) {
        Ok(mut file) => {
            use std::io::Write as _;
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            Ok(token)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => load_connector_token(paths),
        Err(error) => Err(CoreError::Io(error)),
    }
}

/// 读取已创建的 Connector token。
///
/// # Errors
///
/// token 不存在、不可读或为空时返回错误。
pub fn load_connector_token(paths: &AgentFerryPaths) -> Result<String, CoreError> {
    let token = fs::read_to_string(&paths.connector_token)?;
    let token = token.trim();
    if token.is_empty() {
        return Err(CoreError::EmptyConnectorToken);
    }
    Ok(token.to_owned())
}

/// 通过私有 Unix Socket 向 daemon 发送一条已鉴权请求。
///
/// # Errors
///
/// token 读取、socket 连接、framing 或响应解析失败时返回错误。
pub fn send_ipc_request(
    paths: &AgentFerryPaths,
    connector: ConnectorKind,
    request: Value,
) -> Result<HostResponse, CoreError> {
    let mut stream = open_ipc_stream(paths, connector, request)?;
    Ok(read_json_frame(&mut stream)?)
}

/// 打开一条只服务于当前命令的私有 IPC 流。普通命令返回一帧后结束，
/// Handoff 命令可继续返回实时事件；调用方断开只结束观察，不代表取消远端任务。
///
/// # Errors
///
/// token 读取、socket 连接或首帧写入失败时返回错误。
pub fn open_ipc_stream(
    paths: &AgentFerryPaths,
    connector: ConnectorKind,
    request: Value,
) -> Result<UnixStream, CoreError> {
    let token = load_connector_token(paths)?;
    let envelope = IpcEnvelope {
        auth_token: token,
        connector,
        request,
    };
    let mut stream = UnixStream::connect(&paths.socket)?;
    write_json_frame(&mut stream, &envelope)?;
    Ok(stream)
}

/// 读取 Chrome Native Messaging Host manifest。
///
/// # Errors
///
/// 文件读取或 JSON 解析失败时返回错误。
pub fn read_native_host_manifest(path: &Path) -> Result<NativeHostManifest, CoreError> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("无法确定用户目录")]
    HomeDirectoryUnavailable,
    #[error("Connector token 为空")]
    EmptyConnectorToken,
    #[error("文件或 Socket 操作失败: {0}")]
    Io(#[from] io::Error),
    #[error("协议 framing 失败: {0}")]
    Frame(#[from] agent_ferry_protocol::FrameError),
    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root() -> PathBuf {
        env::temp_dir().join(format!("agent-ferry-core-test-{}", Uuid::new_v4()))
    }

    #[test]
    fn connector_token_is_stable_and_private() {
        let root = temporary_root();
        let paths = AgentFerryPaths::from_root(root.clone());
        let first = load_or_create_connector_token(&paths).expect("创建 token");
        let second = load_or_create_connector_token(&paths).expect("读取 token");
        assert_eq!(first, second);

        let mode = fs::metadata(&paths.connector_token)
            .expect("读取 token metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn manifest_accepts_exactly_one_extension() {
        let manifest = NativeHostManifest::new(PathBuf::from("/tmp/agentferry-host"), "abc");
        assert!(manifest.accepts_only_extension("abc"));
        assert!(!manifest.accepts_only_extension("other"));
    }
}
