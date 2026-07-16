use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_ferry_core::AgentFerryPaths;
use serde::{Deserialize, Serialize};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeBinding {
    pub executable: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeState {
    NotDetected,
    NeedsSelection,
    Incompatible,
    NotAuthenticated,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeDiagnosis {
    pub state: ClaudeState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStatus {
    logged_in: bool,
}

#[must_use]
pub fn discover_path_candidates() -> Vec<PathBuf> {
    let Some(path) = env::var_os("PATH") else {
        return Vec::new();
    };
    discover_candidates(env::split_paths(&path).map(|directory| directory.join("claude")))
}

pub fn discover_candidates(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut discovered = Vec::new();
    for candidate in candidates {
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&canonical) else {
            continue;
        };
        if metadata.is_file()
            && metadata.permissions().mode() & 0o111 != 0
            && seen.insert(canonical.clone())
        {
            discovered.push(canonical);
        }
    }
    discovered
}

/// 只通过 Claude 官方 CLI 的公开命令诊断，不读取或复制任何 credential 文件。
#[must_use]
pub fn diagnose_executable(executable: &Path) -> ClaudeDiagnosis {
    let executable = match validate_absolute_executable(executable) {
        Ok(executable) => executable,
        Err(error) => return diagnosis(ClaudeState::Incompatible, error.to_string(), None, None),
    };
    let version_output = match run_with_timeout(&executable, &["--version"]) {
        Ok(output) if output.status.success() => output,
        Ok(_) => {
            return diagnosis(
                ClaudeState::Incompatible,
                "Claude Code --version 返回失败",
                Some(executable),
                None,
            );
        }
        Err(error) => {
            return diagnosis(
                ClaudeState::Incompatible,
                error.to_string(),
                Some(executable),
                None,
            );
        }
    };
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_owned();
    let help = match run_with_timeout(&executable, &["--help"]) {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).into_owned()
        }
        _ => String::new(),
    };
    let required = ["--print", "--output-format", "--permission-mode"];
    if !required.iter().all(|flag| help.contains(flag))
        || !(help.contains("bypassPermissions") || help.contains("--dangerously-skip-permissions"))
    {
        return diagnosis(
            ClaudeState::Incompatible,
            "Claude Code 缺少 Print Mode 或 unrestricted permission flags",
            Some(executable),
            Some(version),
        );
    }
    let auth = match run_with_timeout(&executable, &["auth", "status", "--json"]) {
        Ok(output) if output.status.success() => {
            serde_json::from_slice::<AuthStatus>(&output.stdout)
        }
        _ => {
            return diagnosis(
                ClaudeState::NotAuthenticated,
                "Claude Code auth status 未通过；请先在 Claude Code 官方 CLI 登录",
                Some(executable),
                Some(version),
            );
        }
    };
    if !auth.is_ok_and(|status| status.logged_in) {
        return diagnosis(
            ClaudeState::NotAuthenticated,
            "Claude Code 尚未登录；请先在 Claude Code 官方 CLI 登录",
            Some(executable),
            Some(version),
        );
    }
    diagnosis(
        ClaudeState::Ready,
        "Claude Code Print Mode 与认证检查通过",
        Some(executable),
        Some(version),
    )
}

/// 诊断固定绑定；没有绑定时只报告候选，不修改配置。
///
/// # Errors
///
/// 绑定配置不可读或结构无效时返回错误。
pub fn diagnose_binding(paths: &AgentFerryPaths) -> Result<ClaudeDiagnosis, ClaudeError> {
    if let Some(binding) = load_binding(paths)? {
        return Ok(diagnose_executable(&binding.executable));
    }
    let candidates = discover_path_candidates();
    match candidates.as_slice() {
        [] => Ok(ClaudeDiagnosis {
            state: ClaudeState::NotDetected,
            detail: "未找到 Claude Code；Agent Ferry 不会代为安装".to_owned(),
            executable: None,
            version: None,
            candidates,
        }),
        [candidate] => {
            let mut result = diagnose_executable(candidate);
            result.detail = format!(
                "发现单一候选但尚未绑定：{}；{}",
                candidate.display(),
                result.detail
            );
            result.candidates = candidates;
            Ok(result)
        }
        _ => Ok(ClaudeDiagnosis {
            state: ClaudeState::NeedsSelection,
            detail: "发现多个 Claude Code 候选，请通过绝对路径明确绑定".to_owned(),
            executable: None,
            version: None,
            candidates,
        }),
    }
}

/// 从 PATH 发现 Claude；只有单一兼容且已认证候选时自动保存绑定。
///
/// # Errors
///
/// 读取或保存绑定配置失败时返回错误。
pub fn detect_and_auto_bind(paths: &AgentFerryPaths) -> Result<ClaudeDiagnosis, ClaudeError> {
    if load_binding(paths)?.is_some() {
        return diagnose_binding(paths);
    }
    let candidates = discover_path_candidates();
    if let [candidate] = candidates.as_slice() {
        let diagnosis = diagnose_executable(candidate);
        if diagnosis.state == ClaudeState::Ready {
            save_binding(paths, candidate)?;
        }
        return Ok(diagnosis);
    }
    diagnose_binding(paths)
}

/// 将用户明确选择的绝对路径保存为固定绑定。
///
/// # Errors
///
/// 路径不是绝对可执行文件、版本不兼容或配置保存失败时返回错误。
pub fn bind(paths: &AgentFerryPaths, executable: &Path) -> Result<ClaudeDiagnosis, ClaudeError> {
    let executable = validate_absolute_executable(executable)?;
    let diagnosis = diagnose_executable(&executable);
    if diagnosis.state == ClaudeState::Incompatible {
        return Err(ClaudeError::Incompatible(diagnosis.detail));
    }
    save_binding(paths, &executable)?;
    Ok(diagnosis)
}

/// 读取不包含任何 Claude 凭据的固定可执行路径绑定。
///
/// # Errors
///
/// 配置文件不可读或 JSON 无效时返回错误。
pub fn load_binding(paths: &AgentFerryPaths) -> Result<Option<ClaudeBinding>, ClaudeError> {
    let bytes = match fs::read(&paths.claude_binding) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn save_binding(paths: &AgentFerryPaths, executable: &Path) -> Result<(), ClaudeError> {
    paths.ensure_private_config()?;
    let temporary = paths.claude_binding.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&ClaudeBinding {
        executable: executable.to_owned(),
    })?)?;
    file.sync_all()?;
    fs::rename(&temporary, &paths.claude_binding)?;
    fs::set_permissions(&paths.claude_binding, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn validate_absolute_executable(path: &Path) -> Result<PathBuf, ClaudeError> {
    if !path.is_absolute() {
        return Err(ClaudeError::PathMustBeAbsolute);
    }
    let canonical = path.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(ClaudeError::NotExecutable(canonical));
    }
    Ok(canonical)
}

fn run_with_timeout(executable: &Path, arguments: &[&str]) -> Result<Output, ClaudeError> {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map_err(Into::into);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClaudeError::CommandTimeout);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn diagnosis(
    state: ClaudeState,
    detail: impl Into<String>,
    executable: Option<PathBuf>,
    version: Option<String>,
) -> ClaudeDiagnosis {
    ClaudeDiagnosis {
        state,
        detail: detail.into(),
        executable,
        version,
        candidates: Vec::new(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("Claude Code 必须通过绝对路径绑定")]
    PathMustBeAbsolute,
    #[error("Claude Code 路径不可执行: {0}")]
    NotExecutable(PathBuf),
    #[error("Claude Code 诊断命令超过 5 秒")]
    CommandTimeout,
    #[error("Claude Code 不兼容: {0}")]
    Incompatible(String),
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use uuid::Uuid;

    fn temporary_root() -> PathBuf {
        PathBuf::from(format!("/tmp/af-claude-{}", Uuid::new_v4().simple()))
    }

    fn fake_claude(root: &Path, name: &str, help: &str, logged_in: bool) -> PathBuf {
        fs::create_dir_all(root).expect("创建 fake 目录");
        let path = root.join(name);
        let auth = if logged_in { "true" } else { "false" };
        fs::write(
            &path,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  --version) printf '%s\\n' '2.1.197 (Claude Code)' ;;\n  --help) printf '%s\\n' '{help}' ;;\n  auth) printf '%s\\n' '{{\"loggedIn\":{auth},\"authMethod\":\"test\",\"apiProvider\":\"test\"}}'; [ '{auth}' = true ] ;;\n  *) exit 2 ;;\nesac\n"
            ),
        )
        .expect("写入 fake Claude");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
        path
    }

    #[test]
    fn ready_diagnosis_uses_public_cli_commands_only() {
        let root = temporary_root();
        let executable = fake_claude(
            &root,
            "claude",
            "--print --output-format --permission-mode bypassPermissions",
            true,
        );
        let diagnosis = diagnose_executable(&executable);
        assert_eq!(diagnosis.state, ClaudeState::Ready);
        assert_eq!(diagnosis.version.as_deref(), Some("2.1.197 (Claude Code)"));
        let canonical = executable.canonicalize().expect("规范化路径");
        assert_eq!(diagnosis.executable.as_deref(), Some(canonical.as_path()));
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn distinguishes_incompatible_and_not_authenticated() {
        let root = temporary_root();
        let incompatible = fake_claude(&root, "old-claude", "--print", true);
        assert_eq!(
            diagnose_executable(&incompatible).state,
            ClaudeState::Incompatible
        );
        let logged_out = fake_claude(
            &root,
            "logged-out-claude",
            "--print --output-format --permission-mode --dangerously-skip-permissions",
            false,
        );
        assert_eq!(
            diagnose_executable(&logged_out).state,
            ClaudeState::NotAuthenticated
        );
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn discovery_deduplicates_symlinks_and_binding_is_private() {
        let root = temporary_root();
        let executable = fake_claude(
            &root,
            "claude-real",
            "--print --output-format --permission-mode bypassPermissions",
            true,
        );
        let alias = root.join("claude-alias");
        symlink(&executable, &alias).expect("创建 symlink");
        let canonical = executable.canonicalize().expect("规范化路径");
        let discovered = discover_candidates([executable.clone(), alias]);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0], canonical);

        let paths = AgentFerryPaths::from_root(root.join("ferry"));
        let diagnosis = bind(&paths, &executable).expect("绑定 Claude");
        assert_eq!(diagnosis.state, ClaudeState::Ready);
        let binding = load_binding(&paths).expect("读取绑定").expect("绑定存在");
        assert_eq!(binding.executable, canonical);
        assert_eq!(
            fs::metadata(&paths.claude_binding)
                .expect("读取配置权限")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("清理测试目录");
    }
}
