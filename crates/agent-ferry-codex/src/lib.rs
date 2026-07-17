use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead as _, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_ferry_core::AgentFerryPaths;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const ARTIFACT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_JSON_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_CODEX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexBinding {
    pub executable: PathBuf,
    #[serde(default)]
    pub app_server_supported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSurface {
    Cli,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexState {
    NotDetected,
    NeedsSelection,
    Incompatible,
    NotAuthenticated,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexDiagnosis {
    pub state: CodexState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub cli_supported: bool,
    pub app_server_supported: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexDocument {
    pub title: String,
    pub source_url: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexTaskEvent {
    Started {
        thread_id: String,
        artifact: PathBuf,
    },
    Output(String),
    Tool(String),
    Diagnostic(String),
    Completed(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexTaskResult {
    pub thread_id: String,
    pub artifact: PathBuf,
    pub output: String,
}

#[must_use]
pub fn discover_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join("codex")));
    }
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/codex"));
    }
    // 桌面 App 内置 CLI 不一定进入 LaunchAgent 的 PATH；显式检查官方安装位置才能让 daemon 稳定发现它。
    candidates.extend([
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
    ]);
    discover_candidates(candidates)
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

/// 只调用 Codex 公开 CLI 命令诊断，不读取或复制 Codex 的 auth/state 文件。
#[must_use]
pub fn diagnose_executable(executable: &Path) -> CodexDiagnosis {
    let executable = match validate_absolute_executable(executable) {
        Ok(executable) => executable,
        Err(error) => {
            return diagnosis(
                CodexState::Incompatible,
                error.to_string(),
                None,
                None,
                false,
                false,
            );
        }
    };
    let version_output = match run_with_timeout(&executable, &["--version"]) {
        Ok(output) if output.status.success() => output,
        Ok(_) => {
            return diagnosis(
                CodexState::Incompatible,
                "codex --version 返回失败",
                Some(executable),
                None,
                false,
                false,
            );
        }
        Err(error) => {
            return diagnosis(
                CodexState::Incompatible,
                error.to_string(),
                Some(executable),
                None,
                false,
                false,
            );
        }
    };
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_owned();
    let exec_help = run_with_timeout(&executable, &["exec", "--help"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let cli_supported = [
        "--json",
        "--cd",
        "--dangerously-bypass-approvals-and-sandbox",
    ]
    .iter()
    .all(|flag| exec_help.contains(flag));
    let app_help = run_with_timeout(&executable, &["app-server", "--help"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let app_server_supported = app_help.contains("--stdio") || app_help.contains("stdio://");
    if !cli_supported {
        return diagnosis(
            CodexState::Incompatible,
            "Codex CLI 缺少 exec JSON 或 unrestricted flags",
            Some(executable),
            Some(version),
            false,
            app_server_supported,
        );
    }
    let auth_failure = match run_with_timeout(&executable, &["login", "status"]) {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            Some(format!(
                "退出码 {:?}: {}{}",
                output.status.code(),
                stdout.trim(),
                stderr.trim()
            ))
        }
        Err(error) => Some(error.to_string()),
    };
    if let Some(auth_failure) = auth_failure {
        return diagnosis(
            CodexState::NotAuthenticated,
            format!("Codex 登录检查失败：{auth_failure}；请先运行 codex login"),
            Some(executable),
            Some(version),
            true,
            app_server_supported,
        );
    }
    let detail = if app_server_supported {
        "Codex CLI 与 App Server 检查通过"
    } else {
        "Codex CLI 检查通过；当前版本不支持 App Server"
    };
    diagnosis(
        CodexState::Ready,
        detail,
        Some(executable),
        Some(version),
        true,
        app_server_supported,
    )
}

/// 诊断固定绑定；没有绑定时只报告候选，不修改配置。
///
/// # Errors
///
/// 绑定配置不可读或结构无效时返回错误。
pub fn diagnose_binding(paths: &AgentFerryPaths) -> Result<CodexDiagnosis, CodexError> {
    if let Some(binding) = load_binding(paths)? {
        return Ok(diagnose_executable(&binding.executable));
    }
    let candidates = discover_path_candidates();
    match candidates.as_slice() {
        [] => Ok(CodexDiagnosis {
            state: CodexState::NotDetected,
            detail: "未找到 Codex；Agent Ferry 不会代为安装".to_owned(),
            executable: None,
            version: None,
            cli_supported: false,
            app_server_supported: false,
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
        _ => Ok(CodexDiagnosis {
            state: CodexState::NeedsSelection,
            detail: "发现多个 Codex 候选，请通过绝对路径明确绑定".to_owned(),
            executable: None,
            version: None,
            cli_supported: false,
            app_server_supported: false,
            candidates,
        }),
    }
}

/// 单一兼容候选会自动绑定；Agent Ferry 只保存可执行路径和能力位。
///
/// # Errors
///
/// 读取或保存绑定失败时返回错误。
pub fn detect_and_auto_bind(paths: &AgentFerryPaths) -> Result<CodexDiagnosis, CodexError> {
    if load_binding(paths)?.is_some() {
        return diagnose_binding(paths);
    }
    let candidates = discover_path_candidates();
    if let [candidate] = candidates.as_slice() {
        let diagnosis = diagnose_executable(candidate);
        if diagnosis.state == CodexState::Ready {
            save_binding(paths, candidate, diagnosis.app_server_supported)?;
        }
        return Ok(diagnosis);
    }
    diagnose_binding(paths)
}

/// 保存用户明确选择的 Codex 可执行路径。
///
/// # Errors
///
/// 路径无效、CLI 不兼容或配置写入失败时返回错误。
pub fn bind(paths: &AgentFerryPaths, executable: &Path) -> Result<CodexDiagnosis, CodexError> {
    let executable = validate_absolute_executable(executable)?;
    let diagnosis = diagnose_executable(&executable);
    if diagnosis.state == CodexState::Incompatible {
        return Err(CodexError::Incompatible(diagnosis.detail));
    }
    save_binding(paths, &executable, diagnosis.app_server_supported)?;
    Ok(diagnosis)
}

/// 读取不包含任何 Codex 凭据的固定绑定。
///
/// # Errors
///
/// 配置文件不可读或 JSON 无效时返回错误。
pub fn load_binding(paths: &AgentFerryPaths) -> Result<Option<CodexBinding>, CodexError> {
    let bytes = match fs::read(&paths.codex_binding) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// 在固定 Workspace 中通过 Codex CLI 或 App Server 启动一次全新任务。
///
/// Prompt 只经 stdin/JSON-RPC 发送，正文只写入权限为 0600 的临时 Artifact。
///
/// # Errors
///
/// 绑定、Workspace、输入、子进程或协议无效时返回错误。
pub fn run_task(
    paths: &AgentFerryPaths,
    surface: CodexSurface,
    workspace: &Path,
    prompt: &str,
    document: &CodexDocument,
    mut emit: impl FnMut(CodexTaskEvent),
) -> Result<CodexTaskResult, CodexError> {
    validate_task_input(workspace, prompt, document)?;
    let workspace = workspace.canonicalize()?;
    let binding = load_binding(paths)?.ok_or(CodexError::BindingMissing)?;
    let diagnosis = diagnose_executable(&binding.executable);
    if diagnosis.state != CodexState::Ready {
        return Err(CodexError::NotReady(diagnosis.detail));
    }
    if surface == CodexSurface::App && !diagnosis.app_server_supported {
        return Err(CodexError::AppServerUnsupported);
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let artifact = create_artifact(&task_id, document)?;
    let task_prompt = format!(
        "{prompt}\n\n完整来源内容位于以下只读交接 Artifact，请先读取后再完成任务：\n{}",
        artifact.display()
    );
    let (thread_id, output) = match surface {
        CodexSurface::Cli => execute_cli(
            &binding.executable,
            &workspace,
            &task_id,
            &artifact,
            &task_prompt,
            &mut emit,
        )?,
        CodexSurface::App => execute_app_server(
            &binding.executable,
            &workspace,
            &artifact,
            &task_prompt,
            &mut emit,
        )?,
    };
    emit(CodexTaskEvent::Completed(output.clone()));
    Ok(CodexTaskResult {
        thread_id,
        artifact,
        output,
    })
}

fn validate_task_input(
    workspace: &Path,
    prompt: &str,
    document: &CodexDocument,
) -> Result<(), CodexError> {
    if prompt.trim().is_empty() {
        return Err(CodexError::EmptyPrompt);
    }
    if document.title.trim().is_empty()
        || document.markdown.trim().is_empty()
        || document.markdown.len() > MAX_CODEX_DOCUMENT_BYTES
    {
        return Err(CodexError::InvalidDocument);
    }
    if !workspace.canonicalize()?.is_dir() {
        return Err(CodexError::WorkspaceNotDirectory(workspace.to_owned()));
    }
    Ok(())
}

fn execute_cli(
    executable: &Path,
    workspace: &Path,
    task_id: &str,
    artifact: &Path,
    task_prompt: &str,
    emit: &mut impl FnMut(CodexTaskEvent),
) -> Result<(String, String), CodexError> {
    let mut child = Command::new(executable)
        .args([
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "--skip-git-repo-check",
            "--cd",
        ])
        .arg(workspace)
        .arg("-")
        // `--cd` 约束 Codex 会话；current_dir 同时约束 CLI wrapper、hooks 与启动期配置发现。
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(CodexError::MissingPipe)?
        .write_all(task_prompt.as_bytes())?;
    let stderr_task = read_stderr(child.stderr.take().ok_or(CodexError::MissingPipe)?);
    let mut thread_id = task_id.to_owned();
    let mut started = false;
    let mut output = String::new();
    let mut failure = None;
    let mut item_error = None;
    for line in BufReader::new(child.stdout.take().ok_or(CodexError::MissingPipe)?).lines() {
        let value = parse_json_line(&line?)?;
        match value.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if let Some(id) = value.get("thread_id").and_then(Value::as_str) {
                    id.clone_into(&mut thread_id);
                }
                emit(CodexTaskEvent::Started {
                    thread_id: thread_id.clone(),
                    artifact: artifact.to_owned(),
                });
                started = true;
            }
            Some("item.completed") => {
                if let Some(item) = value.get("item") {
                    match item.get("type").and_then(Value::as_str) {
                        Some("agent_message") => append_output(
                            item.get("text").and_then(Value::as_str),
                            &mut output,
                            emit,
                        ),
                        Some("error") => {
                            item_error = item
                                .get("message")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            if let Some(message) = &item_error {
                                // Codex 会用 error item 传递非终止性配置告警；只在最终没有 Agent 输出时将它升级为失败。
                                emit(CodexTaskEvent::Diagnostic(message.clone()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("turn.failed") => {
                failure = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| Some("Codex CLI Turn 失败".to_owned()));
            }
            _ => {}
        }
    }
    if !started {
        emit(CodexTaskEvent::Started {
            thread_id: thread_id.clone(),
            artifact: artifact.to_owned(),
        });
    }
    if output.is_empty() && failure.is_none() {
        failure = item_error;
    }
    finish_process(child, stderr_task, failure, "Codex CLI", emit)?;
    Ok((thread_id, output))
}

#[allow(clippy::too_many_lines)]
fn execute_app_server(
    executable: &Path,
    workspace: &Path,
    artifact: &Path,
    task_prompt: &str,
    emit: &mut impl FnMut(CodexTaskEvent),
) -> Result<(String, String), CodexError> {
    let mut child = Command::new(executable)
        .args(["app-server", "--stdio"])
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or(CodexError::MissingPipe)?;
    let stderr_task = read_stderr(child.stderr.take().ok_or(CodexError::MissingPipe)?);
    send_json(
        &mut input,
        &json!({
            "method": "initialize", "id": 0,
            "params": {"clientInfo": {"name": "agent_ferry", "title": "Agent Ferry", "version": env!("CARGO_PKG_VERSION")}}
        }),
    )?;

    let mut thread_id = None;
    let mut output = String::new();
    let mut failure = None;
    let stdout = child.stdout.take().ok_or(CodexError::MissingPipe)?;
    for line in BufReader::new(stdout).lines() {
        let value = parse_json_line(&line?)?;
        if value.get("id").and_then(Value::as_i64) == Some(0) {
            if let Some(error) = rpc_error(&value) {
                failure = Some(error);
                break;
            }
            // App Server 要求 initialize 成功后再确认 initialized；提前发送会把版本兼容问题伪装成后续请求失败。
            send_json(&mut input, &json!({"method": "initialized", "params": {}}))?;
            send_json(
                &mut input,
                &json!({
                    "method": "thread/start", "id": 1,
                    "params": {"cwd": workspace, "approvalPolicy": "never", "sandbox": "danger-full-access", "ephemeral": false}
                }),
            )?;
            continue;
        }
        if value.get("id").and_then(Value::as_i64) == Some(1) {
            if let Some(error) = rpc_error(&value) {
                failure = Some(error);
                break;
            }
            let id = value
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .ok_or(CodexError::InvalidAppServerResponse)?
                .to_owned();
            thread_id = Some(id.clone());
            emit(CodexTaskEvent::Started {
                thread_id: id.clone(),
                artifact: artifact.to_owned(),
            });
            send_json(
                &mut input,
                &json!({
                    "method": "turn/start", "id": 2,
                    "params": {"threadId": id, "input": [{"type": "text", "text": task_prompt}]}
                }),
            )?;
            continue;
        }
        if value.get("id").and_then(Value::as_i64) == Some(2) {
            if let Some(error) = rpc_error(&value) {
                failure = Some(error);
                break;
            }
            continue;
        }
        if value.get("id").is_some() && value.get("method").is_some() {
            failure = Some(format!(
                "Codex App Server 请求了无人值守模式不支持的客户端操作: {}",
                value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
            break;
        }
        match value.get("method").and_then(Value::as_str) {
            Some("item/agentMessage/delta") => append_output(
                value.pointer("/params/delta").and_then(Value::as_str),
                &mut output,
                emit,
            ),
            Some("item/started") => {
                if let Some(kind) = value.pointer("/params/item/type").and_then(Value::as_str)
                    && matches!(kind, "commandExecution" | "mcpToolCall" | "webSearch")
                {
                    emit(CodexTaskEvent::Tool(format!("Codex App: {kind}")));
                }
            }
            Some("item/completed") if output.is_empty() => {
                if value.pointer("/params/item/type").and_then(Value::as_str)
                    == Some("agentMessage")
                {
                    append_output(
                        value.pointer("/params/item/text").and_then(Value::as_str),
                        &mut output,
                        emit,
                    );
                }
            }
            Some("turn/completed") => {
                let status = value
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                if status != "completed" {
                    failure = Some(
                        value
                            .pointer("/params/turn/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("Codex App Turn 未完成")
                            .to_owned(),
                    );
                }
                break;
            }
            _ => {}
        }
    }
    drop(input);
    let thread_id = thread_id.ok_or(CodexError::InvalidAppServerResponse)?;
    finish_process(child, stderr_task, failure, "Codex App Server", emit)?;
    Ok((thread_id, output))
}

fn append_output(text: Option<&str>, output: &mut String, emit: &mut impl FnMut(CodexTaskEvent)) {
    if let Some(text) = text
        && !text.is_empty()
    {
        output.push_str(text);
        emit(CodexTaskEvent::Output(text.to_owned()));
    }
}

fn rpc_error(value: &Value) -> Option<String> {
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn send_json(writer: &mut impl Write, value: &Value) -> Result<(), CodexError> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn parse_json_line(line: &str) -> Result<Value, CodexError> {
    if line.len() > MAX_JSON_LINE_BYTES {
        return Err(CodexError::OutputLineTooLarge);
    }
    serde_json::from_str(line).map_err(Into::into)
}

fn read_stderr(stderr: impl Read + Send + 'static) -> thread::JoinHandle<std::io::Result<String>> {
    thread::spawn(move || {
        let mut text = String::new();
        BufReader::new(stderr)
            .read_to_string(&mut text)
            .map(|_| text)
    })
}

fn finish_process(
    mut child: std::process::Child,
    stderr_task: thread::JoinHandle<std::io::Result<String>>,
    failure: Option<String>,
    label: &str,
    emit: &mut impl FnMut(CodexTaskEvent),
) -> Result<(), CodexError> {
    let status = child.wait()?;
    let stderr = stderr_task
        .join()
        .map_err(|_| CodexError::StderrReaderPanicked)??;
    if !stderr.trim().is_empty() {
        emit(CodexTaskEvent::Diagnostic(stderr.trim().to_owned()));
    }
    if !status.success() || failure.is_some() {
        let detail = failure.unwrap_or_else(|| stderr.trim().to_owned());
        let message = format!("{label} 退出码 {:?}: {detail}", status.code());
        emit(CodexTaskEvent::Failed(message.clone()));
        return Err(CodexError::TaskFailed(message));
    }
    Ok(())
}

fn create_artifact(task_id: &str, document: &CodexDocument) -> Result<PathBuf, CodexError> {
    let root = env::temp_dir().join("agent-ferry").join("artifacts");
    fs::create_dir_all(&root)?;
    fs::set_permissions(
        root.parent().ok_or(CodexError::InvalidArtifactRoot)?,
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
            // 只清理超过保留期的旧输入；当前任务文件尚未创建，不会影响运行中 Agent。
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn save_binding(
    paths: &AgentFerryPaths,
    executable: &Path,
    app_server_supported: bool,
) -> Result<(), CodexError> {
    paths.ensure_private_config()?;
    let temporary = paths
        .codex_binding
        .with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&CodexBinding {
        executable: executable.to_owned(),
        app_server_supported,
    })?)?;
    file.sync_all()?;
    fs::rename(&temporary, &paths.codex_binding)?;
    fs::set_permissions(&paths.codex_binding, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn validate_absolute_executable(path: &Path) -> Result<PathBuf, CodexError> {
    if !path.is_absolute() {
        return Err(CodexError::PathMustBeAbsolute);
    }
    let canonical = path.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(CodexError::NotExecutable(canonical));
    }
    Ok(canonical)
}

fn run_with_timeout(executable: &Path, arguments: &[&str]) -> Result<Output, CodexError> {
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
            return Err(CodexError::CommandTimeout);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn diagnosis(
    state: CodexState,
    detail: impl Into<String>,
    executable: Option<PathBuf>,
    version: Option<String>,
    cli_supported: bool,
    app_server_supported: bool,
) -> CodexDiagnosis {
    CodexDiagnosis {
        state,
        detail: detail.into(),
        executable,
        version,
        cli_supported,
        app_server_supported,
        candidates: Vec::new(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("Codex 必须通过绝对路径绑定")]
    PathMustBeAbsolute,
    #[error("Codex 路径不可执行: {0}")]
    NotExecutable(PathBuf),
    #[error("Codex 诊断命令超过 5 秒")]
    CommandTimeout,
    #[error("尚未绑定 Codex；请先运行 aferry agent codex detect")]
    BindingMissing,
    #[error("Codex 尚未 ready: {0}")]
    NotReady(String),
    #[error("当前 Codex 不支持 App Server")]
    AppServerUnsupported,
    #[error("Prompt 不能为空")]
    EmptyPrompt,
    #[error("文档标题、正文或大小无效")]
    InvalidDocument,
    #[error("Workspace 不是目录: {0}")]
    WorkspaceNotDirectory(PathBuf),
    #[error("无法创建系统临时 Artifact 根目录")]
    InvalidArtifactRoot,
    #[error("Codex 未提供所需 stdio 管道")]
    MissingPipe,
    #[error("Codex JSON 单行超过 1 MiB")]
    OutputLineTooLarge,
    #[error("Codex stderr 读取线程异常退出")]
    StderrReaderPanicked,
    #[error("Codex App Server 返回了无效响应")]
    InvalidAppServerResponse,
    #[error("Codex Task 失败: {0}")]
    TaskFailed(String),
    #[error("Codex 不兼容: {0}")]
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

    fn temporary_root(name: &str) -> PathBuf {
        env::temp_dir().join(format!("af-codex-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn fake_codex(root: &Path) -> PathBuf {
        fs::create_dir_all(root).expect("创建 fake Codex 目录");
        let executable = root.join("codex");
        fs::write(
            &executable,
            format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo 'codex-cli 0.test'; exit 0; fi
if [ "$1" = "login" ]; then echo 'Logged in using test'; exit 0; fi
if [ "$1" = "exec" ] && [ "$2" = "--help" ]; then echo '--json --cd --dangerously-bypass-approvals-and-sandbox'; exit 0; fi
if [ "$1" = "app-server" ] && [ "$2" = "--help" ]; then echo '--stdio stdio://'; exit 0; fi
if [ "$1" = "exec" ]; then
  pwd > '{0}/cli-cwd'
  printf '%s\n' "$@" > '{0}/cli-args'
  cat > '{0}/cli-stdin'
  echo '{{"type":"thread.started","thread_id":"cli-thread"}}'
  echo '{{"type":"item.completed","item":{{"type":"error","message":"非终止告警"}}}}'
  echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"CODEX_CLI_TEST_OK"}}}}'
  echo '{{"type":"turn.completed"}}'
  exit 0
fi
if [ "$1" = "app-server" ]; then
  IFS= read -r initialize
  printf '%s\n' "$initialize" > '{0}/app-messages'
  echo '{{"id":0,"result":{{"userAgent":"fake"}}}}'
  IFS= read -r initialized
  IFS= read -r thread_start
  printf '%s\n%s\n' "$initialized" "$thread_start" >> '{0}/app-messages'
  echo '{{"id":1,"result":{{"thread":{{"id":"app-thread"}}}}}}'
  IFS= read -r turn_start
  printf '%s\n' "$turn_start" >> '{0}/app-messages'
  echo '{{"method":"item/agentMessage/delta","params":{{"threadId":"app-thread","turnId":"turn-1","itemId":"item-1","delta":"CODEX_APP_TEST_OK"}}}}'
  echo '{{"method":"turn/completed","params":{{"threadId":"app-thread","turn":{{"id":"turn-1","items":[],"status":"completed"}}}}}}'
  exit 0
fi
exit 1
"#,
                root.display()
            ),
        )
        .expect("写入 fake Codex");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("设置 fake Codex 执行权限");
        executable
    }

    fn fixture() -> (PathBuf, AgentFerryPaths, PathBuf, PathBuf) {
        let root = temporary_root("task");
        let executable = fake_codex(&root.join("fake"));
        let paths = AgentFerryPaths::from_root(root.join("ferry"));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).expect("创建 Workspace");
        bind(&paths, &executable).expect("绑定 fake Codex");
        (root, paths, workspace, executable)
    }

    fn document() -> CodexDocument {
        CodexDocument {
            title: "测试文档".to_owned(),
            source_url: "https://example.com/article".to_owned(),
            markdown: "# 正文\n\n完整页面内容".to_owned(),
        }
    }

    #[test]
    fn diagnosis_reports_both_surfaces_and_saves_private_binding() {
        let (root, paths, _workspace, executable) = fixture();
        let diagnosis = diagnose_executable(&executable);
        assert_eq!(diagnosis.state, CodexState::Ready);
        assert!(diagnosis.cli_supported);
        assert!(diagnosis.app_server_supported);
        let binding = load_binding(&paths).expect("读取绑定").expect("绑定存在");
        assert!(binding.app_server_supported);
        assert_eq!(
            fs::metadata(&paths.codex_binding)
                .expect("读取绑定权限")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn cli_task_uses_stdin_and_keeps_non_terminal_error_as_diagnostic() {
        let (root, paths, workspace, _executable) = fixture();
        let mut events = Vec::new();
        let result = run_task(
            &paths,
            CodexSurface::Cli,
            &workspace,
            "分析正文",
            &document(),
            |event| events.push(event),
        )
        .expect("CLI 任务成功");
        assert_eq!(result.thread_id, "cli-thread");
        assert_eq!(result.output, "CODEX_CLI_TEST_OK");
        assert!(events.iter().any(
            |event| matches!(event, CodexTaskEvent::Diagnostic(text) if text == "非终止告警")
        ));
        let args = fs::read_to_string(root.join("fake/cli-args")).expect("读取 CLI argv");
        assert!(args.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(!args.contains("分析正文"));
        let stdin = fs::read_to_string(root.join("fake/cli-stdin")).expect("读取 CLI stdin");
        assert!(stdin.contains("分析正文"));
        assert!(stdin.contains("完整来源内容位于以下只读交接 Artifact"));
        assert_eq!(
            fs::metadata(&result.artifact)
                .expect("读取 Artifact 权限")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("清理测试目录");
    }

    #[test]
    fn app_task_follows_handshake_and_creates_persistent_unrestricted_thread() {
        let (root, paths, workspace, _executable) = fixture();
        let result = run_task(
            &paths,
            CodexSurface::App,
            &workspace,
            "分析正文",
            &document(),
            |_| {},
        )
        .expect("App Server 任务成功");
        assert_eq!(result.thread_id, "app-thread");
        assert_eq!(result.output, "CODEX_APP_TEST_OK");
        let messages =
            fs::read_to_string(root.join("fake/app-messages")).expect("读取 App Server 消息");
        let lines = messages.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("initialize"));
        assert!(lines[1].contains("initialized"));
        assert!(lines[2].contains("thread/start"));
        assert!(lines[2].contains("danger-full-access"));
        assert!(lines[2].contains("\"approvalPolicy\":\"never\""));
        assert!(lines[2].contains("\"ephemeral\":false"));
        assert!(lines[3].contains("turn/start"));
        assert!(lines[3].contains("分析正文"));
        fs::remove_dir_all(root).expect("清理测试目录");
    }
}
