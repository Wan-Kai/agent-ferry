use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use agent_ferry_core::{AgentFerryPaths, NativeHostManifest};
#[cfg(debug_assertions)]
use agent_ferry_hermes::DevelopmentCredentialStore;
#[cfg(not(debug_assertions))]
use agent_ferry_hermes::KeychainCredentialStore;
use agent_ferry_hermes::{CredentialStore as _, load_connections};
use serde::Serialize;
use thiserror::Error;

use crate::service::{ServiceManager, ServiceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalState {
    Removed,
    NotFound,
    PreservedForeign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDataState {
    Preserved,
    Purged,
}

#[derive(Debug, Serialize)]
pub struct UninstallReport {
    pub service: RemovalState,
    pub native_host: RemovalState,
    pub program: RemovalState,
    pub commands_removed: usize,
    pub credentials_removed: usize,
    pub temporary_artifacts: RemovalState,
    pub user_data: UserDataState,
    pub logs: UserDataState,
}

pub fn run(purge: bool, yes: bool) -> Result<UninstallReport, UninstallError> {
    if purge && !yes {
        return Err(UninstallError::PurgeConfirmationRequired);
    }
    if yes && !purge {
        return Err(UninstallError::YesWithoutPurge);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(UninstallError::HomeDirectoryUnavailable)?;
    let install_root = home.join(".local/share/agent-ferry");
    let data_root = home.join(".agent-ferry");
    let log_root = home.join("Library/Logs/Agent Ferry");
    let artifact_root = env::temp_dir().join("agent-ferry/artifacts");
    let paths = AgentFerryPaths::discover()?;
    let _lock = LifecycleLock::acquire(&home)?;

    ensure_removable_directory(&install_root)?;
    ensure_removable_artifact_directory(&artifact_root)?;
    if purge {
        ensure_removable_directory(&data_root)?;
        ensure_removable_directory(&log_root)?;
    }

    let manager = ServiceManager::discover()?;
    let plist = home.join("Library/LaunchAgents/com.agentferry.daemon.plist");
    let service_owned = service_is_owned(&plist, &install_root, &home)?;
    let service_was_loaded = if service_owned {
        let status = manager.status()?;
        let loaded = status.state != ServiceState::Stopped;
        manager.stop()?;
        loaded
    } else {
        false
    };

    let credentials_removed = if purge {
        match purge_credentials(&paths) {
            Ok(count) => count,
            Err(error) => {
                if service_owned && service_was_loaded {
                    let _ = manager.start();
                }
                return Err(error);
            }
        }
    } else {
        0
    };

    let service = if service_owned {
        manager.uninstall()?;
        RemovalState::Removed
    } else if plist.try_exists()? {
        RemovalState::PreservedForeign
    } else {
        RemovalState::NotFound
    };
    let native_host = remove_owned_native_host(&paths.native_host_manifest, &install_root, &home)?;
    let commands_removed = remove_owned_command_links(&home, &install_root)?;
    let program = remove_program_root(&install_root)?;
    let temporary_artifacts = remove_artifact_directory(&artifact_root)?;

    if purge {
        remove_directory_if_present(&data_root)?;
        remove_directory_if_present(&log_root)?;
    }

    Ok(UninstallReport {
        service,
        native_host,
        program,
        commands_removed,
        credentials_removed,
        temporary_artifacts,
        user_data: if purge {
            UserDataState::Purged
        } else {
            UserDataState::Preserved
        },
        logs: if purge {
            UserDataState::Purged
        } else {
            UserDataState::Preserved
        },
    })
}

/**
 * 本地 Agent 的临时正文可能包含用户浏览的完整页面。它不属于可恢复配置，因此普通卸载也应
 * 清除；同时必须逐级拒绝符号链接，避免固定的临时目录名被替换后让卸载越界删除其他位置。
 */
fn ensure_removable_artifact_directory(path: &Path) -> Result<(), UninstallError> {
    if let Some(namespace) = path.parent() {
        ensure_removable_directory(namespace)?;
    }
    ensure_removable_directory(path)
}

fn remove_artifact_directory(path: &Path) -> Result<RemovalState, UninstallError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemovalState::NotFound);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(UninstallError::UnsafeRemovalPath(path.to_path_buf()));
    }
    fs::remove_dir_all(path)?;
    if let Some(namespace) = path.parent() {
        match fs::remove_dir(namespace) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(RemovalState::Removed)
}

fn service_is_owned(
    plist: &Path,
    install_root: &Path,
    home: &Path,
) -> Result<bool, UninstallError> {
    let metadata = match fs::symlink_metadata(plist) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(false);
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "ProgramArguments.0", "raw", "-o", "-"])
        .arg(plist)
        .output()?;
    if !output.status.success() {
        return Ok(false);
    }
    let program = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let legacy = home
        .join("Library/Application Support/Agent Ferry/bin")
        .join("agentferryd");
    Ok(
        (program.starts_with(install_root) && program.file_name() == Some("agentferryd".as_ref()))
            || program == legacy,
    )
}

fn remove_owned_native_host(
    manifest_path: &Path,
    install_root: &Path,
    home: &Path,
) -> Result<RemovalState, UninstallError> {
    let metadata = match fs::symlink_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemovalState::NotFound);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(RemovalState::PreservedForeign);
    }
    let manifest: NativeHostManifest = match serde_json::from_slice(&fs::read(manifest_path)?) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(RemovalState::PreservedForeign),
    };
    let legacy = home
        .join("Library/Application Support/Agent Ferry/bin")
        .join("agentferry-host");
    let owned = manifest.name == agent_ferry_protocol::NATIVE_HOST_NAME
        && manifest.path.file_name() == Some("agentferry-host".as_ref())
        && (manifest.path.starts_with(install_root) || manifest.path == legacy);
    if !owned {
        return Ok(RemovalState::PreservedForeign);
    }
    fs::remove_file(manifest_path)?;
    Ok(RemovalState::Removed)
}

fn remove_owned_command_links(home: &Path, install_root: &Path) -> Result<usize, UninstallError> {
    let mut removed = 0;
    for binary in ["aferry", "agentferryd", "agentferry-host"] {
        let command = home.join(".local/bin").join(binary);
        let metadata = match fs::symlink_metadata(&command) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink()
            && fs::read_link(&command)? == install_root.join("current/bin").join(binary)
        {
            fs::remove_file(command)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn remove_program_root(install_root: &Path) -> Result<RemovalState, UninstallError> {
    match fs::symlink_metadata(install_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            UninstallError::UnsafeRemovalPath(install_root.to_path_buf()),
        ),
        Ok(_) => {
            fs::remove_dir_all(install_root)?;
            Ok(RemovalState::Removed)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RemovalState::NotFound),
        Err(error) => Err(error.into()),
    }
}

fn ensure_removable_directory(path: &Path) -> Result<(), UninstallError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(UninstallError::UnsafeRemovalPath(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_directory_if_present(path: &Path) -> Result<(), UninstallError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn purge_credentials(paths: &AgentFerryPaths) -> Result<usize, UninstallError> {
    let connections = load_connections(&paths.hermes_connections)?;
    #[cfg(debug_assertions)]
    let store = DevelopmentCredentialStore::new(paths.development_credentials.clone());
    #[cfg(not(debug_assertions))]
    let store = KeychainCredentialStore;
    for connection in &connections.connections {
        store.delete(&connection.credential_ref)?;
    }
    Ok(connections.connections.len())
}

struct LifecycleLock {
    path: PathBuf,
}

impl LifecycleLock {
    fn acquire(home: &Path) -> Result<Self, UninstallError> {
        let parent = home.join(".local/share");
        fs::create_dir_all(&parent)?;
        let path = parent.join(".agent-ferry.lock");
        fs::create_dir(&path).map_err(|source| UninstallError::LifecycleLocked {
            path: path.clone(),
            source,
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self { path })
    }
}

impl Drop for LifecycleLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[derive(Debug, Error)]
pub enum UninstallError {
    #[error("彻底删除配置、历史、日志和凭据需要显式执行 aferry uninstall --purge --yes")]
    PurgeConfirmationRequired,
    #[error("--yes 只能与 --purge 一起使用")]
    YesWithoutPurge,
    #[error("无法确定用户目录")]
    HomeDirectoryUnavailable,
    #[error("拒绝删除不是普通目录的路径：{0}")]
    UnsafeRemovalPath(PathBuf),
    #[error("另一个安装、更新或卸载正在执行，无法获取生命周期锁 {path}: {source}")]
    LifecycleLocked {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Hermes(#[from] agent_ferry_hermes::HermesError),
    #[error(transparent)]
    Service(#[from] crate::service::ServiceError),
    #[error("文件操作失败: {0}")]
    Io(#[from] std::io::Error),
}
