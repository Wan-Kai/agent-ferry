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
const ARTIFACT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_OUTPUT_LINE_BYTES: usize = 1024 * 1024;
pub const DEFAULT_OPENCODE_MODEL: &str = "deepseek/deepseek-chat";
pub const MAX_OPENCODE_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeBinding {
    pub executable: PathBuf,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeState {
    NotDetected,
    NeedsSelection,
    Incompatible,
    ModelUnavailable,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenCodeDiagnosis {
    pub state: OpenCodeState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeDocument {
    pub title: String,
    pub source_url: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCodeTaskEvent {
    Started { task_id: String, artifact: PathBuf },
    Output(String),
    Tool(String),
    Diagnostic(String),
    Completed(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeTaskResult {
    pub task_id: String,
    pub artifact: PathBuf,
    pub output: String,
}

#[must_use]
pub fn discover_path_candidates() -> Vec<PathBuf> {
    let Some(path) = env::var_os("PATH") else {
        return Vec::new();
    };
    discover_candidates(env::split_paths(&path).map(|directory| directory.join("opencode")))
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

/// 只调用 `OpenCode` 的公开只读命令检查版本、run flags 和显式模型，不读取凭据文件。
#[must_use]
pub fn diagnose_executable(executable: &Path, model: &str) -> OpenCodeDiagnosis {
    let executable = match validate_absolute_executable(executable) {
        Ok(executable) => executable,
        Err(error) => {
            return diagnosis(
                OpenCodeState::Incompatible,
                error.to_string(),
                None,
                None,
                Some(model.to_owned()),
            );
        }
    };
    let Some((provider, model_name)) = valid_model(model) else {
        return diagnosis(
            OpenCodeState::ModelUnavailable,
            "OpenCode model 必须使用 provider/model 格式",
            Some(executable),
            None,
            Some(model.to_owned()),
        );
    };
    let version_output = match run_with_timeout(&executable, &["--version"]) {
        Ok(output) if output.status.success() => output,
        Ok(_) => {
            return diagnosis(
                OpenCodeState::Incompatible,
                "OpenCode --version 返回失败",
                Some(executable),
                None,
                Some(model.to_owned()),
            );
        }
        Err(error) => {
            return diagnosis(
                OpenCodeState::Incompatible,
                error.to_string(),
                Some(executable),
                None,
                Some(model.to_owned()),
            );
        }
    };
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_owned();
    let help = run_with_timeout(&executable, &["run", "--help"])
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(String::new, |output| command_text(&output));
    let required = ["--format", "--file", "--model", "--dir", "--auto"];
    if !required.iter().all(|flag| help.contains(flag)) {
        return diagnosis(
            OpenCodeState::Incompatible,
            "OpenCode 缺少 run 所需的 JSON、文件、模型、目录或 auto flags",
            Some(executable),
            Some(version),
            Some(model.to_owned()),
        );
    }
    let models = match run_with_timeout(&executable, &["models", provider]) {
        Ok(output) if output.status.success() => command_text(&output),
        _ => {
            return diagnosis(
                OpenCodeState::ModelUnavailable,
                format!("无法读取 OpenCode provider {provider} 的模型列表"),
                Some(executable),
                Some(version),
                Some(model.to_owned()),
            );
        }
    };
    let expected = format!("{provider}/{model_name}");
    if !models.lines().any(|line| line.trim() == expected) {
        return diagnosis(
            OpenCodeState::ModelUnavailable,
            format!("OpenCode 未列出显式模型 {model}"),
            Some(executable),
            Some(version),
            Some(model.to_owned()),
        );
    }
    diagnosis(
        OpenCodeState::Ready,
        format!("OpenCode 与显式模型 {model} 检查通过；认证将在真实任务中验证"),
        Some(executable),
        Some(version),
        Some(model.to_owned()),
    )
}

/// 诊断已保存的可执行路径与显式模型绑定。
///
/// # Errors
///
/// 绑定配置不可读或 JSON 无效时返回错误。
pub fn diagnose_binding(paths: &AgentFerryPaths) -> Result<OpenCodeDiagnosis, OpenCodeError> {
    if let Some(binding) = load_binding(paths)? {
        return Ok(diagnose_executable(&binding.executable, &binding.model));
    }
    let candidates = discover_path_candidates();
    match candidates.as_slice() {
        [] => Ok(OpenCodeDiagnosis {
            state: OpenCodeState::NotDetected,
            detail: "未找到 OpenCode；Agent Ferry 不会代为安装".to_owned(),
            executable: None,
            version: None,
            model: None,
            candidates,
        }),
        [candidate] => {
            let mut result = diagnose_executable(candidate, DEFAULT_OPENCODE_MODEL);
            result.detail = format!(
                "发现单一候选但尚未绑定：{}；{}",
                candidate.display(),
                result.detail
            );
            result.candidates = candidates;
            Ok(result)
        }
        _ => Ok(OpenCodeDiagnosis {
            state: OpenCodeState::NeedsSelection,
            detail: "发现多个 OpenCode 候选，请通过绝对路径明确绑定".to_owned(),
            executable: None,
            version: None,
            model: Some(DEFAULT_OPENCODE_MODEL.to_owned()),
            candidates,
        }),
    }
}

/// 从 PATH 发现 OpenCode；只有单一兼容且包含显式模型的候选才自动保存绑定。
///
/// # Errors
///
/// 读取或保存绑定配置失败时返回错误。
pub fn detect_and_auto_bind(
    paths: &AgentFerryPaths,
    model: &str,
) -> Result<OpenCodeDiagnosis, OpenCodeError> {
    if load_binding(paths)?.is_some() {
        return diagnose_binding(paths);
    }
    let candidates = discover_path_candidates();
    if let [candidate] = candidates.as_slice() {
        let result = diagnose_executable(candidate, model);
        if result.state == OpenCodeState::Ready {
            save_binding(paths, candidate, model)?;
        }
        return Ok(result);
    }
    diagnose_binding(paths)
}

/// 保存用户明确选择的 `OpenCode` 可执行路径和模型。
///
/// # Errors
///
/// 路径、模型或配置无效时返回错误。
pub fn bind(
    paths: &AgentFerryPaths,
    executable: &Path,
    model: &str,
) -> Result<OpenCodeDiagnosis, OpenCodeError> {
    let executable = validate_absolute_executable(executable)?;
    let result = diagnose_executable(&executable, model);
    if result.state != OpenCodeState::Ready {
        return Err(OpenCodeError::NotReady(result.detail));
    }
    save_binding(paths, &executable, model)?;
    Ok(result)
}

/// 读取不包含任何第三方凭据的 `OpenCode` 绑定。
///
/// # Errors
///
/// 配置文件不可读或 JSON 无效时返回错误。
pub fn load_binding(paths: &AgentFerryPaths) -> Result<Option<OpenCodeBinding>, OpenCodeError> {
    let bytes = match fs::read(&paths.opencode_binding) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// 在指定 Workspace 启动全新的 `OpenCode` 一次性任务。
///
/// Prompt 只走 stdin，正文只通过系统临时 Artifact 和 `--file` 交给 `OpenCode`。
///
/// # Errors
///
/// 绑定、Workspace、正文、进程或 JSON 输出无效时返回错误。
pub fn run_task(
    paths: &AgentFerryPaths,
    workspace: &Path,
    prompt: &str,
    document: &OpenCodeDocument,
    mut emit: impl FnMut(OpenCodeTaskEvent),
) -> Result<OpenCodeTaskResult, OpenCodeError> {
    if prompt.trim().is_empty() {
        return Err(OpenCodeError::EmptyPrompt);
    }
    if document.title.trim().is_empty()
        || document.markdown.trim().is_empty()
        || document.markdown.len() > MAX_OPENCODE_DOCUMENT_BYTES
    {
        return Err(OpenCodeError::InvalidDocument);
    }
    let workspace = workspace.canonicalize()?;
    if !workspace.is_dir() {
        return Err(OpenCodeError::WorkspaceNotDirectory(workspace));
    }
    let binding = load_binding(paths)?.ok_or(OpenCodeError::BindingMissing)?;
    let result = diagnose_executable(&binding.executable, &binding.model);
    if result.state != OpenCodeState::Ready {
        return Err(OpenCodeError::NotReady(result.detail));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let artifact = create_artifact(&task_id, document)?;
    emit(OpenCodeTaskEvent::Started {
        task_id: task_id.clone(),
        artifact: artifact.clone(),
    });
    let output = execute_process(&binding, &workspace, &artifact, prompt, &mut emit)?;
    emit(OpenCodeTaskEvent::Completed(output.clone()));
    Ok(OpenCodeTaskResult {
        task_id,
        artifact,
        output,
    })
}

fn execute_process(
    binding: &OpenCodeBinding,
    workspace: &Path,
    artifact: &Path,
    prompt: &str,
    emit: &mut impl FnMut(OpenCodeTaskEvent),
) -> Result<String, OpenCodeError> {
    let mut child = Command::new(&binding.executable)
        .current_dir(workspace)
        .args(["run", "--auto", "--format", "json", "--model"])
        .arg(&binding.model)
        .arg("--file")
        .arg(artifact)
        .arg("--dir")
        .arg(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(OpenCodeError::MissingPipe)?
        .write_all(prompt.as_bytes())?;
    let stderr = child.stderr.take().ok_or(OpenCodeError::MissingPipe)?;
    let stderr_task = thread::spawn(move || {
        let mut text = String::new();
        BufReader::new(stderr)
            .read_to_string(&mut text)
            .map(|_| text)
    });
    let stdout = child.stdout.take().ok_or(OpenCodeError::MissingPipe)?;
    let mut final_output = String::new();
    let mut structured_error = None;
    for line in BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error.into());
            }
        };
        if line.len() > MAX_OUTPUT_LINE_BYTES {
            terminate_child(&mut child);
            return Err(OpenCodeError::OutputLineTooLarge);
        }
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error.into());
            }
        };
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                if let Some(text) = value
                    .pointer("/part/text")
                    .and_then(serde_json::Value::as_str)
                {
                    final_output.push_str(text);
                    emit(OpenCodeTaskEvent::Output(text.to_owned()));
                }
            }
            Some("tool_use") => {
                let tool = value
                    .pointer("/part/tool")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                let status = value
                    .pointer("/part/state/status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                emit(OpenCodeTaskEvent::Tool(format!("{tool}: {status}")));
            }
            Some("error") => {
                structured_error = Some(
                    value
                        .pointer("/error/data/message")
                        .or_else(|| value.pointer("/error/message"))
                        .or_else(|| value.get("error"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("OpenCode 返回未说明原因的错误")
                        .to_owned(),
                );
            }
            _ => {}
        }
    }
    let status = child.wait()?;
    let stderr = stderr_task
        .join()
        .map_err(|_| OpenCodeError::StderrReaderPanicked)??;
    if !stderr.trim().is_empty() {
        emit(OpenCodeTaskEvent::Diagnostic(stderr.trim().to_owned()));
    }
    if !status.success() || structured_error.is_some() {
        let detail = structured_error.unwrap_or_else(|| stderr.trim().to_owned());
        let message = format!("OpenCode 退出码 {:?}: {detail}", status.code());
        emit(OpenCodeTaskEvent::Failed(message.clone()));
        return Err(OpenCodeError::TaskFailed(message));
    }
    Ok(final_output)
}

fn terminate_child(child: &mut std::process::Child) {
    // 输出协议破损时进程可能仍在等待工具或网络；立即回收，避免后台遗留无观察者任务。
    let _ = child.kill();
    let _ = child.wait();
}

fn create_artifact(task_id: &str, document: &OpenCodeDocument) -> Result<PathBuf, OpenCodeError> {
    let root = env::temp_dir().join("agent-ferry").join("artifacts");
    fs::create_dir_all(&root)?;
    fs::set_permissions(
        root.parent().ok_or(OpenCodeError::InvalidArtifactRoot)?,
        fs::Permissions::from_mode(0o700),
    )?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    cleanup_expired_artifacts(&root);
    let artifact = root.join(format!("{task_id}.md"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&artifact)?;
    write!(
        file,
        "# {}\n\n- 来源：{}\n- Agent Ferry Task：{}\n\n---\n\n{}",
        document.title, document.source_url, task_id, document.markdown
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
            // 只清理终态任务留下的过期输入；本次 Artifact 此时尚不存在，不会删到运行中任务。
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn save_binding(
    paths: &AgentFerryPaths,
    executable: &Path,
    model: &str,
) -> Result<(), OpenCodeError> {
    paths.ensure_private_config()?;
    let temporary = paths.opencode_binding.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&OpenCodeBinding {
        executable: executable.to_owned(),
        model: model.to_owned(),
    })?)?;
    file.sync_all()?;
    fs::rename(&temporary, &paths.opencode_binding)?;
    fs::set_permissions(&paths.opencode_binding, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn valid_model(model: &str) -> Option<(&str, &str)> {
    let (provider, name) = model.split_once('/')?;
    (!provider.trim().is_empty() && !name.trim().is_empty()).then_some((provider, name))
}

fn command_text(output: &Output) -> String {
    // OpenCode 不同版本会把 help 写入 stdout 或 stderr；诊断只读取两条公开输出并统一匹配。
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn validate_absolute_executable(path: &Path) -> Result<PathBuf, OpenCodeError> {
    if !path.is_absolute() {
        return Err(OpenCodeError::PathMustBeAbsolute);
    }
    let canonical = path.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(OpenCodeError::NotExecutable(canonical));
    }
    Ok(canonical)
}

fn run_with_timeout(executable: &Path, arguments: &[&str]) -> Result<Output, OpenCodeError> {
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
            return Err(OpenCodeError::CommandTimeout);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn diagnosis(
    state: OpenCodeState,
    detail: impl Into<String>,
    executable: Option<PathBuf>,
    version: Option<String>,
    model: Option<String>,
) -> OpenCodeDiagnosis {
    OpenCodeDiagnosis {
        state,
        detail: detail.into(),
        executable,
        version,
        model,
        candidates: Vec::new(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenCodeError {
    #[error("OpenCode 必须通过绝对路径绑定")]
    PathMustBeAbsolute,
    #[error("OpenCode 路径不可执行: {0}")]
    NotExecutable(PathBuf),
    #[error("OpenCode 诊断命令超过 5 秒")]
    CommandTimeout,
    #[error("尚未绑定 OpenCode；请先运行 aferry agent opencode detect")]
    BindingMissing,
    #[error("OpenCode 尚未 ready: {0}")]
    NotReady(String),
    #[error("Prompt 不能为空")]
    EmptyPrompt,
    #[error("文档标题、正文或大小无效")]
    InvalidDocument,
    #[error("Workspace 不是目录: {0}")]
    WorkspaceNotDirectory(PathBuf),
    #[error("无法创建系统临时 Artifact 根目录")]
    InvalidArtifactRoot,
    #[error("OpenCode 未提供所需 stdio 管道")]
    MissingPipe,
    #[error("OpenCode JSON 单行超过 1 MiB")]
    OutputLineTooLarge,
    #[error("OpenCode stderr 读取线程异常退出")]
    StderrReaderPanicked,
    #[error("OpenCode 任务失败: {0}")]
    TaskFailed(String),
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
