use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-codex-cli-{}", Uuid::new_v4().simple()))
}

fn fake_codex(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).expect("创建 fake bin");
    let executable = directory.join("codex");
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
  echo '{{"type":"thread.started","thread_id":"cli-e2e-thread"}}'
  echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"CLI_CODEX_EXEC_OK"}}}}'
  echo '{{"type":"turn.completed"}}'
  exit 0
fi
if [ "$1" = "app-server" ]; then
  IFS= read -r initialize
  echo '{{"id":0,"result":{{"userAgent":"fake"}}}}'
  IFS= read -r initialized
  IFS= read -r thread_start
  printf '%s\n%s\n%s\n' "$initialize" "$initialized" "$thread_start" > '{0}/app-messages'
  echo '{{"id":1,"result":{{"thread":{{"id":"app-e2e-thread"}}}}}}'
  IFS= read -r turn_start
  printf '%s\n' "$turn_start" >> '{0}/app-messages'
  echo '{{"method":"item/agentMessage/delta","params":{{"threadId":"app-e2e-thread","turnId":"turn-1","itemId":"item-1","delta":"CLI_CODEX_APP_OK"}}}}'
  echo '{{"method":"turn/completed","params":{{"threadId":"app-e2e-thread","turn":{{"id":"turn-1","items":[],"status":"completed"}}}}}}'
  exit 0
fi
exit 1
"#,
            directory.display()
        ),
    )
    .expect("写入 fake Codex");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

fn bind_fake(ferry_home: &Path, executable: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", ferry_home)
        .args([
            "agent",
            "codex",
            "bind",
            "--path",
            executable.to_str().expect("Codex 路径 UTF-8"),
            "--json",
        ])
        .output()
        .expect("绑定 fake Codex");
    assert!(
        output.status.success(),
        "绑定失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("绑定 stdout UTF-8");
    assert!(stdout.contains("\"cli_supported\": true"));
    assert!(stdout.contains("\"app_server_supported\": true"));
}

fn run_surface(root: &Path, surface: &str, expected: &str) {
    let ferry_home = root.join("ferry");
    let executable = fake_codex(&root.join("bin"));
    bind_fake(&ferry_home, &executable);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("创建 Workspace");
    let document = root.join("document.md");
    fs::write(&document, "Codex CLI E2E 完整文档正文").expect("写入文档");
    let private_prompt = "只允许通过安全输入通道发送的 Codex Prompt";
    let mut run = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &ferry_home)
        .args([
            "agent",
            "codex",
            "run",
            "--surface",
            surface,
            "--workspace",
            workspace.to_str().expect("Workspace UTF-8"),
            "--document-file",
            document.to_str().expect("文档路径 UTF-8"),
            "--title",
            "Codex CLI E2E 文档",
            "--source-url",
            "https://example.com/codex-cli-e2e",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 Codex Task");
    run.stdin
        .take()
        .expect("获取 stdin")
        .write_all(private_prompt.as_bytes())
        .expect("写入 Prompt");
    let output = run.wait_with_output().expect("等待 Codex Task");
    assert!(
        output.status.success(),
        "Codex Task 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("Codex Task 已启动"));
    assert!(stdout.contains(expected));
    assert!(stdout.contains("Codex Task 已完成"));
}

#[test]
fn codex_exec_cli_command_completes() {
    let root = temporary_root();
    run_surface(&root, "cli", "CLI_CODEX_EXEC_OK");
    let args = fs::read_to_string(root.join("bin/cli-args")).expect("读取 CLI argv");
    assert!(args.contains("--dangerously-bypass-approvals-and-sandbox"));
    assert!(!args.contains("只允许通过安全输入通道"));
    let stdin = fs::read_to_string(root.join("bin/cli-stdin")).expect("读取 CLI stdin");
    assert!(stdin.contains("只允许通过安全输入通道发送的 Codex Prompt"));
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn codex_app_cli_command_completes() {
    let root = temporary_root();
    run_surface(&root, "app", "CLI_CODEX_APP_OK");
    let messages = fs::read_to_string(root.join("bin/app-messages")).expect("读取 App Server 消息");
    assert!(messages.contains("thread/start"));
    assert!(messages.contains("danger-full-access"));
    assert!(messages.contains("turn/start"));
    assert!(messages.contains("只允许通过安全输入通道发送的 Codex Prompt"));
    fs::remove_dir_all(root).expect("清理测试目录");
}
