use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_ferry_core::AgentFerryPaths;
use serde::{Deserialize, Serialize};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_CLAUDE_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const ARTIFACT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeDocument {
    pub title: String,
    pub source_url: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeTaskEvent {
    Started {
        session_id: String,
        artifact: PathBuf,
    },
    Output(String),
    Diagnostic(String),
    Completed(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeTaskResult {
    pub session_id: String,
    pub artifact: PathBuf,
    pub output: String,
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

/// 在指定 Workspace 中启动全新的 unrestricted Claude Print Task。
///
/// Prompt 只通过 stdin 发送；正文只写入系统临时 Artifact，二者都不会进入 argv。
///
/// # Errors
///
/// 绑定不可用、Workspace/正文无效、进程启动失败、stream-json 无效或 Claude 非零退出时返回错误。
pub fn run_print_task(
    paths: &AgentFerryPaths,
    workspace: &Path,
    prompt: &str,
    document: &ClaudeDocument,
    mut emit: impl FnMut(ClaudeTaskEvent),
) -> Result<ClaudeTaskResult, ClaudeError> {
    if prompt.trim().is_empty() {
        return Err(ClaudeError::EmptyPrompt);
    }
    if document.title.trim().is_empty()
        || document.markdown.trim().is_empty()
        || document.markdown.len() > MAX_CLAUDE_DOCUMENT_BYTES
    {
        return Err(ClaudeError::InvalidDocument);
    }
    let workspace = workspace.canonicalize()?;
    if !workspace.is_dir() {
        return Err(ClaudeError::WorkspaceNotDirectory(workspace));
    }
    let binding = load_binding(paths)?.ok_or(ClaudeError::BindingMissing)?;
    let diagnosis = diagnose_executable(&binding.executable);
    if diagnosis.state != ClaudeState::Ready {
        return Err(ClaudeError::NotReady(diagnosis.detail));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let artifact = create_artifact(&session_id, document)?;
    emit(ClaudeTaskEvent::Started {
        session_id: session_id.clone(),
        artifact: artifact.clone(),
    });
    let task_prompt = format!(
        "{prompt}\n\n完整来源内容位于以下只读交接 Artifact，请先读取后再完成任务：\n{}",
        artifact.display()
    );
    let final_output = execute_print_process(
        &binding.executable,
        &workspace,
        &session_id,
        &task_prompt,
        &mut emit,
    )?;
    emit(ClaudeTaskEvent::Completed(final_output.clone()));
    Ok(ClaudeTaskResult {
        session_id,
        artifact,
        output: final_output,
    })
}

fn execute_print_process(
    executable: &Path,
    workspace: &Path,
    session_id: &str,
    task_prompt: &str,
    emit: &mut impl FnMut(ClaudeTaskEvent),
) -> Result<String, ClaudeError> {
    let mut child = Command::new(executable)
        .current_dir(workspace)
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--session-id",
            session_id,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(ClaudeError::MissingPipe)?
        .write_all(task_prompt.as_bytes())?;
    let stderr = child.stderr.take().ok_or(ClaudeError::MissingPipe)?;
    let stderr_task = thread::spawn(move || {
        let mut text = String::new();
        BufReader::new(stderr)
            .read_to_string(&mut text)
            .map(|_| text)
    });
    let stdout = child.stdout.take().ok_or(ClaudeError::MissingPipe)?;
    let mut final_output = String::new();
    let mut structured_error = None;
    for line in BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        if line.len() > 1024 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClaudeError::OutputLineTooLarge);
        }
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        if let Some(delta) = stream_text_delta(&value) {
            final_output.push_str(delta);
            emit(ClaudeTaskEvent::Output(delta.to_owned()));
        } else if value.get("type").and_then(serde_json::Value::as_str) == Some("result") {
            let result = value
                .get("result")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Claude 返回了未说明原因的错误");
            if value
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                structured_error = Some(result.to_owned());
            } else if final_output.is_empty() {
                final_output.push_str(result);
            }
        }
    }
    let status = child.wait()?;
    let stderr = stderr_task
        .join()
        .map_err(|_| ClaudeError::StderrReaderPanicked)??;
    if !stderr.trim().is_empty() {
        emit(ClaudeTaskEvent::Diagnostic(stderr.trim().to_owned()));
    }
    if !status.success() || structured_error.is_some() {
        let detail = structured_error.unwrap_or_else(|| stderr.trim().to_owned());
        let message = format!("Claude Code 退出码 {:?}: {detail}", status.code());
        emit(ClaudeTaskEvent::Failed(message.clone()));
        return Err(ClaudeError::TaskFailed(message));
    }
    Ok(final_output)
}

fn create_artifact(session_id: &str, document: &ClaudeDocument) -> Result<PathBuf, ClaudeError> {
    let root = env::temp_dir().join("agent-ferry").join("artifacts");
    fs::create_dir_all(&root)?;
    fs::set_permissions(
        root.parent().ok_or(ClaudeError::InvalidArtifactRoot)?,
        fs::Permissions::from_mode(0o700),
    )?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    cleanup_expired_artifacts(&root);
    let artifact = root.join(format!("{session_id}.md"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&artifact)?;
    write!(
        file,
        "# {}\n\n- 来源：{}\n- Agent Ferry Session：{}\n\n---\n\n{}",
        document.title, document.source_url, session_id, document.markdown
    )?;
    file.sync_all()?;
    Ok(artifact)
}

fn cleanup_expired_artifacts(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let expired = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= ARTIFACT_RETENTION);
        if metadata.is_file() && expired {
            // 只清理上一次已终结任务留下的老文件；本次 Artifact 尚未创建，不可能误删运行中输入。
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn stream_text_delta(value: &serde_json::Value) -> Option<&str> {
    (value.get("type")?.as_str()? == "stream_event")
        .then_some(())
        .and_then(|()| value.get("event"))?
        .get("delta")?
        .get("text")?
        .as_str()
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
    #[error("尚未绑定 Claude Code；请先运行 aferry agent claude detect")]
    BindingMissing,
    #[error("Claude Code 尚未 ready: {0}")]
    NotReady(String),
    #[error("Prompt 不能为空")]
    EmptyPrompt,
    #[error("文档标题、正文或大小无效")]
    InvalidDocument,
    #[error("Workspace 不是目录: {0}")]
    WorkspaceNotDirectory(PathBuf),
    #[error("无法创建系统临时 Artifact 根目录")]
    InvalidArtifactRoot,
    #[error("Claude Code 未提供所需 stdio 管道")]
    MissingPipe,
    #[error("Claude stream-json 单行超过 1 MiB")]
    OutputLineTooLarge,
    #[error("Claude stderr 读取线程异常退出")]
    StderrReaderPanicked,
    #[error("Claude Print Task 失败: {0}")]
    TaskFailed(String),
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

    fn fake_print_claude(root: &Path, exit_code: i32, structured_error: bool) -> PathBuf {
        fs::create_dir_all(root).expect("创建 fake 目录");
        let path = root.join("claude-print");
        let cwd_log = root.join("cwd.log");
        let args_log = root.join("args.log");
        let stdin_log = root.join("stdin.log");
        let result = if structured_error {
            r#"{"type":"result","is_error":true,"result":"API Error: fake forbidden"}"#
        } else {
            r#"{"type":"result","result":"第一段第二段"}"#
        };
        fs::write(
            &path,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  --version) echo '2.1.197 (Claude Code)'; exit 0 ;;\n  --help) echo '--print --output-format --permission-mode --dangerously-skip-permissions'; exit 0 ;;\n  auth) echo '{{\"loggedIn\":true}}'; exit 0 ;;\nesac\npwd > '{}'\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf '%s\\n' '{{\"type\":\"stream_event\",\"event\":{{\"delta\":{{\"text\":\"第一段\"}}}}}}'\nprintf '%s\\n' '{{\"type\":\"stream_event\",\"event\":{{\"delta\":{{\"text\":\"第二段\"}}}}}}'\nprintf '%s\\n' '{result}'\necho 'fake diagnostic' >&2\nexit {exit_code}\n",
                cwd_log.display(),
                args_log.display(),
                stdin_log.display()
            ),
        )
        .expect("写入 fake Print Claude");
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

    #[test]
    fn print_task_uses_fixed_cwd_structured_argv_stdin_and_temp_artifact() {
        let root = temporary_root();
        let executable = fake_print_claude(&root.join("fake"), 0, false);
        let paths = AgentFerryPaths::from_root(root.join("ferry"));
        bind(&paths, &executable).expect("绑定 fake Claude");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("创建 Workspace");
        let document = ClaudeDocument {
            title: "完整文章".to_owned(),
            source_url: "https://example.com/article".to_owned(),
            markdown: "不可丢失的完整正文".repeat(200),
        };
        let mut events = Vec::new();
        let result = run_print_task(
            &paths,
            &workspace,
            "请分析文章并输出结论",
            &document,
            |event| events.push(event),
        )
        .expect("运行 Print Task");

        assert_eq!(result.output, "第一段第二段");
        assert!(result.artifact.starts_with(env::temp_dir()));
        assert!(!result.artifact.starts_with(&workspace));
        let artifact = fs::read_to_string(&result.artifact).expect("读取 Artifact");
        assert!(artifact.contains("https://example.com/article"));
        assert!(artifact.ends_with(&document.markdown));
        assert_eq!(
            fs::metadata(&result.artifact)
                .expect("读取 Artifact 权限")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let fake_root = root.join("fake");
        assert_eq!(
            fs::read_to_string(fake_root.join("cwd.log"))
                .expect("读取 cwd")
                .trim(),
            workspace
                .canonicalize()
                .expect("规范化 Workspace")
                .to_string_lossy()
        );
        let args = fs::read_to_string(fake_root.join("args.log")).expect("读取 argv");
        for required in [
            "--print",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--session-id",
            &result.session_id,
        ] {
            assert!(args.lines().any(|argument| argument == required));
        }
        assert!(!args.contains("--bare"));
        assert!(!args.contains("--no-session-persistence"));
        assert!(!args.contains("请分析文章"));
        assert!(!args.contains("不可丢失"));
        let stdin = fs::read_to_string(fake_root.join("stdin.log")).expect("读取 stdin");
        assert!(stdin.contains("请分析文章并输出结论"));
        assert!(stdin.contains(result.artifact.to_string_lossy().as_ref()));
        assert!(!stdin.contains("不可丢失的完整正文"));
        assert!(matches!(
            events.first(),
            Some(ClaudeTaskEvent::Started { .. })
        ));
        assert!(matches!(events.last(), Some(ClaudeTaskEvent::Completed(_))));
        assert!(events.iter().any(
            |event| matches!(event, ClaudeTaskEvent::Diagnostic(text) if text == "fake diagnostic")
        ));

        let _ = fs::remove_file(result.artifact);
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn print_task_surfaces_structured_result_error() {
        let root = temporary_root();
        let executable = fake_print_claude(&root.join("fake"), 0, true);
        let paths = AgentFerryPaths::from_root(root.join("ferry"));
        bind(&paths, &executable).expect("绑定 fake Claude");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("创建 Workspace");
        let mut events = Vec::new();
        let error = run_print_task(
            &paths,
            &workspace,
            "执行失败测试",
            &ClaudeDocument {
                title: "失败文档".to_owned(),
                source_url: "https://example.com/failure".to_owned(),
                markdown: "足够的失败测试正文".repeat(20),
            },
            |event| events.push(event),
        )
        .expect_err("结构化错误必须失败");
        assert!(matches!(error, ClaudeError::TaskFailed(_)));
        assert!(error.to_string().contains("API Error: fake forbidden"));
        assert!(matches!(events.last(), Some(ClaudeTaskEvent::Failed(_))));
        fs::remove_dir_all(root).expect("清理测试目录");
    }
}
