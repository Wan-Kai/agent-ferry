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

/// 返回普通用户数据的默认目录。
///
/// CLI 本体使用 `~/.local` 安装，但配置、历史和运行状态需要跨版本保留，因此使用独立且稳定的
/// `~/.agent-ferry`。该函数显式接收 home，避免测试通过修改进程环境变量造成并发污染。
#[must_use]
pub fn default_data_root(home: &Path) -> PathBuf {
    home.join(".agent-ferry")
}

/// 返回早期开发版本使用的数据目录。
#[must_use]
pub fn legacy_data_root(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Agent Ferry")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataMigrationOutcome {
    NotNeeded,
    Migrated { from: PathBuf, to: PathBuf },
}

/// 将早期开发版本的数据根目录原子迁移到 CLI 风格目录。
///
/// 迁移只在“旧目录存在、新目录不存在”时发生。双目录合并无法判断同名配置哪一份更新，静默覆盖会
/// 丢失用户连接和历史，因此将冲突交给上层明确展示。两个目录都位于同一用户 home 下，使用 rename
/// 可以避免复制中断留下半份数据；安装器必须在停止旧 daemon 后调用本函数。
///
/// # Errors
///
/// 双目录同时存在或文件系统操作失败时返回错误。
pub fn migrate_legacy_data(home: &Path) -> Result<DataMigrationOutcome, CoreError> {
    let legacy = legacy_data_root(home);
    let current = default_data_root(home);
    let legacy_exists = safe_data_root_exists(&legacy)?;
    let current_exists = safe_data_root_exists(&current)?;

    match (legacy_exists, current_exists) {
        (false, _) => Ok(DataMigrationOutcome::NotNeeded),
        (true, true) => Err(CoreError::DataRootConflict { legacy, current }),
        (true, false) => {
            fs::rename(&legacy, &current)?;
            fs::set_permissions(&current, fs::Permissions::from_mode(0o700))?;
            Ok(DataMigrationOutcome::Migrated {
                from: legacy,
                to: current,
            })
        }
    }
}

fn safe_data_root_exists(path: &Path) -> Result<bool, CoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(CoreError::UnsafeDataRoot {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFerryPaths {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub hermes_connections: PathBuf,
    pub claude_binding: PathBuf,
    pub codex_binding: PathBuf,
    pub opencode_binding: PathBuf,
    pub workspaces: PathBuf,
    pub history_dir: PathBuf,
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
            default_data_root(Path::new(&home))
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
            codex_binding: config_dir.join("codex.json"),
            opencode_binding: config_dir.join("opencode.json"),
            workspaces: config_dir.join("workspaces.json"),
            history_dir: root.join("history"),
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
    #[error("新旧数据目录同时存在，拒绝自动合并: {legacy} 与 {current}")]
    DataRootConflict { legacy: PathBuf, current: PathBuf },
    #[error("数据目录不是普通目录，拒绝迁移: {path}")]
    UnsafeDataRoot { path: PathBuf },
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
    use std::os::unix::fs::symlink;

    fn temporary_root() -> PathBuf {
        env::temp_dir().join(format!("agent-ferry-core-test-{}", Uuid::new_v4()))
    }

    #[test]
    fn default_data_root_uses_cli_style_location() {
        let home = temporary_root();
        assert_eq!(default_data_root(&home), home.join(".agent-ferry"));
        assert_eq!(
            legacy_data_root(&home),
            home.join("Library")
                .join("Application Support")
                .join("Agent Ferry")
        );
    }

    #[test]
    fn legacy_data_is_moved_atomically_when_new_root_is_absent() {
        let home = temporary_root();
        let legacy = legacy_data_root(&home);
        fs::create_dir_all(legacy.join("config")).expect("创建旧配置目录");
        fs::write(legacy.join("config/workspaces.json"), b"legacy").expect("写入旧配置");

        let outcome = migrate_legacy_data(&home).expect("迁移旧数据");
        let current = default_data_root(&home);

        assert_eq!(
            outcome,
            DataMigrationOutcome::Migrated {
                from: legacy.clone(),
                to: current.clone(),
            }
        );
        assert!(!legacy.exists());
        assert_eq!(
            fs::read(current.join("config/workspaces.json")).expect("读取迁移后的配置"),
            b"legacy"
        );
        let mode = fs::metadata(&current)
            .expect("读取新目录权限")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);

        fs::remove_dir_all(home).expect("清理测试目录");
    }

    #[test]
    fn migration_refuses_to_merge_two_existing_data_roots() {
        let home = temporary_root();
        let legacy = legacy_data_root(&home);
        let current = default_data_root(&home);
        fs::create_dir_all(&legacy).expect("创建旧目录");
        fs::create_dir_all(&current).expect("创建新目录");
        fs::write(legacy.join("legacy.txt"), b"legacy").expect("写入旧数据");
        fs::write(current.join("current.txt"), b"current").expect("写入新数据");

        let error = migrate_legacy_data(&home).expect_err("双目录并存必须拒绝迁移");

        assert!(matches!(
            error,
            CoreError::DataRootConflict {
                legacy: ref actual_legacy,
                current: ref actual_current,
            } if actual_legacy == &legacy && actual_current == &current
        ));
        assert_eq!(
            fs::read(legacy.join("legacy.txt")).expect("旧数据仍存在"),
            b"legacy"
        );
        assert_eq!(
            fs::read(current.join("current.txt")).expect("新数据仍存在"),
            b"current"
        );

        fs::remove_dir_all(home).expect("清理测试目录");
    }

    #[test]
    fn migration_refuses_a_legacy_root_symbolic_link() {
        let home = temporary_root();
        let outside = temporary_root();
        fs::create_dir_all(&outside).expect("创建链接目标目录");
        let legacy = legacy_data_root(&home);
        fs::create_dir_all(legacy.parent().expect("旧目录包含父目录")).expect("创建旧目录父级");
        symlink(&outside, &legacy).expect("创建旧目录符号链接");

        let error = migrate_legacy_data(&home).expect_err("符号链接必须拒绝迁移");

        assert!(matches!(
            error,
            CoreError::UnsafeDataRoot { ref path } if path == &legacy
        ));
        assert!(legacy.exists());
        assert!(outside.exists());

        fs::remove_file(legacy).expect("清理符号链接");
        fs::remove_dir_all(home).expect("清理测试 home");
        fs::remove_dir_all(outside).expect("清理链接目标");
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
