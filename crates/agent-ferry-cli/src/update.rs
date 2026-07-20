use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use thiserror::Error;

const RELEASE_MANIFEST_PREFIX: &str = "https://github.com/Wan-Kai/agent-ferry/releases/download";

#[derive(Debug, Deserialize)]
struct InstallRecord {
    manifest_url: String,
}

pub fn run(version: Option<&str>, manifest_url: Option<&str>) -> Result<i32, UpdateError> {
    if version.is_some() && manifest_url.is_some() {
        return Err(UpdateError::ConflictingSelectors);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(UpdateError::HomeDirectoryUnavailable)?;
    let installer = packaged_installer()?;
    validate_installer(&installer)?;
    let selected_manifest = select_manifest(&home, version, manifest_url)?;

    // 更新器只执行随当前已校验 archive 安装的脚本，manifest URL 作为独立 argv 传递；这样不会把
    // 网络内容或 URL 解释为 shell，也避免每次更新重新下载一段尚未验证的安装逻辑。
    let status = Command::new("/bin/bash")
        .arg(&installer)
        .arg("--manifest-url")
        .arg(&selected_manifest)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn packaged_installer() -> Result<PathBuf, UpdateError> {
    // `~/.local/bin/aferry` 是稳定入口，实际文件位于版本目录；先解析链接才能从真实版本根找到与
    // 当前二进制一同校验、安装的脚本，不能误把用户 PATH 目录当成可信程序目录。
    let executable = fs::canonicalize(env::current_exe()?)?;
    let bin_directory = executable
        .parent()
        .ok_or(UpdateError::InstallLayoutUnavailable)?;
    let version_root = bin_directory
        .parent()
        .ok_or(UpdateError::InstallLayoutUnavailable)?;
    Ok(version_root.join("share").join("install.sh"))
}

fn validate_installer(installer: &Path) -> Result<(), UpdateError> {
    let metadata =
        fs::symlink_metadata(installer).map_err(|source| UpdateError::InstallerUnavailable {
            path: installer.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(UpdateError::UnsafeInstallerType(installer.to_path_buf()));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(UpdateError::InstallerNotExecutable(installer.to_path_buf()));
    }
    if mode & 0o022 != 0 {
        return Err(UpdateError::InstallerWritableByOthers(
            installer.to_path_buf(),
        ));
    }
    Ok(())
}

fn select_manifest(
    home: &Path,
    version: Option<&str>,
    manifest_url: Option<&str>,
) -> Result<String, UpdateError> {
    if let Some(manifest_url) = manifest_url {
        return validate_manifest_url(manifest_url);
    }
    if let Some(version) = version {
        if !valid_version(version) {
            return Err(UpdateError::InvalidVersion(version.to_owned()));
        }
        return Ok(format!(
            "{RELEASE_MANIFEST_PREFIX}/v{version}/release-manifest.json"
        ));
    }
    let record_path = home.join(".agent-ferry").join("install.json");
    let record: InstallRecord =
        serde_json::from_slice(&fs::read(&record_path).map_err(|source| {
            UpdateError::InstallRecordUnavailable {
                path: record_path.clone(),
                source,
            }
        })?)?;
    validate_manifest_url(&record.manifest_url)
}

fn validate_manifest_url(value: &str) -> Result<String, UpdateError> {
    if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(UpdateError::InvalidManifestUrl);
    }
    if !value.starts_with("https://") && !value.starts_with("file://") {
        return Err(UpdateError::InvalidManifestUrl);
    }
    Ok(value.to_owned())
}

fn valid_version(value: &str) -> bool {
    let core = value.split(['-', '.']).collect::<Vec<_>>();
    core.len() >= 3
        && core[..3]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("--version 与 --manifest-url 不能同时使用")]
    ConflictingSelectors,
    #[error("无法确定用户目录")]
    HomeDirectoryUnavailable,
    #[error("当前 aferry 不在完整的版本化安装布局中，无法找到受信任安装器")]
    InstallLayoutUnavailable,
    #[error("无法读取受信任安装器 {path}: {source}")]
    InstallerUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("受信任安装器必须是普通文件，不能是符号链接：{0}")]
    UnsafeInstallerType(PathBuf),
    #[error("受信任安装器不可执行：{0}")]
    InstallerNotExecutable(PathBuf),
    #[error("受信任安装器不能被 group 或 other 写入：{0}")]
    InstallerWritableByOthers(PathBuf),
    #[error("版本号无效：{0}")]
    InvalidVersion(String),
    #[error("manifest URL 必须是无控制字符的 https:// 或显式 file:// URL")]
    InvalidManifestUrl,
    #[error("无法读取安装记录 {path}: {source}")]
    InstallRecordUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("安装记录 JSON 无效: {0}")]
    InvalidInstallRecord(#[from] serde_json::Error),
    #[error("无法运行受信任安装器: {0}")]
    Io(#[from] std::io::Error),
}
