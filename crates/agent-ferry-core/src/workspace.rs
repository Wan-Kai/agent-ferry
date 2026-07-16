use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{AgentFerryPaths, CoreError};

const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            workspaces: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Ready,
    Missing,
    NotDirectory,
    NotWritable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceDiagnosis {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub state: WorkspaceState,
    pub detail: String,
}

/// 读取 Workspace 配置；首次使用且文件不存在时返回空配置。
///
/// # Errors
///
/// 文件不可读、JSON 无效或版本不兼容时返回错误。
pub fn load(paths: &AgentFerryPaths) -> Result<WorkspaceConfig, WorkspaceError> {
    let bytes = match fs::read(&paths.workspaces) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceConfig::default());
        }
        Err(error) => return Err(error.into()),
    };
    let config: WorkspaceConfig = serde_json::from_slice(&bytes)?;
    if config.version != CONFIG_VERSION {
        return Err(WorkspaceError::UnsupportedVersion(config.version));
    }
    Ok(config)
}

/// 保存一个用户已经创建的目录；Ferry 不创建 Workspace 本身。
///
/// # Errors
///
/// 名称、目录、重复约束或配置写入无效时返回错误。
pub fn add(
    paths: &AgentFerryPaths,
    name: &str,
    directory: &Path,
) -> Result<Workspace, WorkspaceError> {
    validate_name(name)?;
    let canonical = directory
        .canonicalize()
        .map_err(|_| WorkspaceError::DirectoryMissing(directory.to_owned()))?;
    if !canonical.is_dir() {
        return Err(WorkspaceError::NotDirectory(canonical));
    }
    let mut config = load(paths)?;
    if config.workspaces.iter().any(|item| item.name == name) {
        return Err(WorkspaceError::DuplicateName(name.to_owned()));
    }
    if config.workspaces.iter().any(|item| item.path == canonical) {
        return Err(WorkspaceError::DuplicatePath(canonical));
    }
    let workspace = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_owned(),
        path: canonical,
    };
    config.workspaces.push(workspace.clone());
    save(paths, &config)?;
    Ok(workspace)
}

/// 按 ID 或名称移除配置引用，不会删除真实目录或目录内文件。
///
/// # Errors
///
/// 配置不可读、目标不存在或配置无法保存时返回错误。
pub fn remove(paths: &AgentFerryPaths, identifier: &str) -> Result<Workspace, WorkspaceError> {
    let mut config = load(paths)?;
    let index = config
        .workspaces
        .iter()
        .position(|item| item.id == identifier || item.name == identifier)
        .ok_or_else(|| WorkspaceError::NotFound(identifier.to_owned()))?;
    let removed = config.workspaces.remove(index);
    save(paths, &config)?;
    Ok(removed)
}

#[must_use]
pub fn diagnose(workspace: &Workspace) -> WorkspaceDiagnosis {
    let (state, detail) = match fs::metadata(&workspace.path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (WorkspaceState::Missing, "目录已不存在")
        }
        Err(_) => (WorkspaceState::Missing, "目录无法访问"),
        Ok(metadata) if !metadata.is_dir() => (WorkspaceState::NotDirectory, "路径不再是目录"),
        Ok(metadata) if metadata.permissions().readonly() => {
            (WorkspaceState::NotWritable, "目录没有可写权限")
        }
        Ok(_) => (WorkspaceState::Ready, "Workspace 可用"),
    };
    WorkspaceDiagnosis {
        id: workspace.id.clone(),
        name: workspace.name.clone(),
        path: workspace.path.clone(),
        state,
        detail: detail.to_owned(),
    }
}

fn save(paths: &AgentFerryPaths, config: &WorkspaceConfig) -> Result<(), WorkspaceError> {
    paths.ensure_private_config()?;
    let temporary = paths.workspaces.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(config)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &paths.workspaces)?;
    fs::set_permissions(&paths.workspaces, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn validate_name(name: &str) -> Result<(), WorkspaceError> {
    if name.is_empty()
        || name.len() > 128
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(WorkspaceError::InvalidName);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Workspace 名称不能为空、不能包含首尾空白或控制字符，且最多 128 字节")]
    InvalidName,
    #[error("Workspace 目录不存在: {0}")]
    DirectoryMissing(PathBuf),
    #[error("Workspace 路径不是目录: {0}")]
    NotDirectory(PathBuf),
    #[error("Workspace 名称已存在: {0}")]
    DuplicateName(String),
    #[error("Workspace 路径已存在: {0}")]
    DuplicatePath(PathBuf),
    #[error("未找到 Workspace: {0}")]
    NotFound(String),
    #[error("不支持的 Workspace 配置版本: {0}")]
    UnsupportedVersion(u32),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
