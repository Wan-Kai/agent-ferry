use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-cc-cli-{}", Uuid::new_v4().simple()))
}

fn fake_claude(directory: &Path, logged_in: bool) -> PathBuf {
    fs::create_dir_all(directory).expect("创建 fake bin");
    let executable = directory.join("claude");
    let auth = if logged_in { "true" } else { "false" };
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo '2.1.197 (Claude Code)' ;;\n  --help) echo '--print --output-format --permission-mode bypassPermissions' ;;\n  auth) echo '{{\"loggedIn\":{auth},\"authMethod\":\"test\",\"apiProvider\":\"test\"}}'; [ '{auth}' = true ] ;;\n  *) exit 2 ;;\nesac\n"
        ),
    )
    .expect("写入 fake Claude");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

fn fake_print_claude(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).expect("创建 fake bin");
    let executable = directory.join("claude");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo '2.1.197 (Claude Code)'; exit 0 ;;\n  --help) echo '--print --output-format --permission-mode --dangerously-skip-permissions'; exit 0 ;;\n  auth) echo '{{\"loggedIn\":true}}'; exit 0 ;;\nesac\npwd > '{}/cwd'\nprintf '%s\\n' \"$@\" > '{}/args'\ncat > '{}/stdin'\necho '{{\"type\":\"stream_event\",\"event\":{{\"delta\":{{\"text\":\"真实进程输出\"}}}}}}'\necho '{{\"type\":\"result\",\"result\":\"真实进程输出\"}}'\n",
            directory.display(),
            directory.display(),
            directory.display()
        ),
    )
    .expect("写入 fake Print Claude");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

#[test]
fn missing_claude_only_prints_official_guidance() {
    let root = temporary_root();
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .env("PATH", root.join("empty-bin"))
        .args(["agent", "claude", "detect"])
        .output()
        .expect("运行 Claude detect");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("not_detected"));
    assert!(stdout.contains("https://code.claude.com/docs/en/installation"));
    assert!(stdout.contains("aferry agent claude detect"));
    assert!(!root.join("config/claude-code.json").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_ready_candidate_auto_binds_absolute_canonical_path() {
    let root = temporary_root();
    let executable = fake_claude(&root.join("bin"), true);
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .env("PATH", root.join("bin"))
        .args(["agent", "claude", "detect", "--json"])
        .output()
        .expect("运行 Claude detect");
    assert!(
        output.status.success(),
        "detect 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"state\": \"ready\""));
    let canonical = executable.canonicalize().expect("规范化路径");
    assert!(stdout.contains(canonical.to_string_lossy().as_ref()));
    let config = root.join("ferry/config/claude-code.json");
    assert!(
        fs::read_to_string(&config)
            .expect("读取绑定")
            .contains("claude")
    );
    assert_eq!(
        fs::metadata(config).expect("读取权限").permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn multiple_candidates_require_explicit_absolute_selection() {
    let root = temporary_root();
    let first = root.join("first");
    let second = root.join("second");
    fake_claude(&first, true);
    fake_claude(&second, true);
    let path = std::env::join_paths([first, second]).expect("拼接 PATH");
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .env("PATH", path)
        .args(["agent", "claude", "detect", "--json"])
        .output()
        .expect("运行 Claude detect");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"state\": \"needs_selection\""));
    assert!(!root.join("ferry/config/claude-code.json").exists());
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn cli_print_task_streams_output_and_keeps_prompt_out_of_argv() {
    let root = temporary_root();
    let fake_bin = root.join("bin");
    fake_print_claude(&fake_bin);
    let ferry_home = root.join("ferry");
    let detect = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &ferry_home)
        .env("PATH", &fake_bin)
        .args(["agent", "claude", "detect", "--json"])
        .output()
        .expect("绑定 fake Claude");
    assert!(detect.status.success());

    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("创建 Workspace");
    let document = root.join("document.md");
    fs::write(&document, "CLI E2E 完整文档正文").expect("写入文档");
    let private_prompt = "只允许通过 stdin 发送的 Prompt";
    let mut run = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &ferry_home)
        .args([
            "agent",
            "claude",
            "run",
            "--workspace",
            workspace.to_str().expect("Workspace UTF-8"),
            "--document-file",
            document.to_str().expect("文档路径 UTF-8"),
            "--title",
            "CLI E2E 文档",
            "--source-url",
            "https://example.com/cli-e2e",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 CLI Print Task");
    run.stdin
        .take()
        .expect("获取 stdin")
        .write_all(private_prompt.as_bytes())
        .expect("写入 Prompt");
    let output = run.wait_with_output().expect("等待 CLI Print Task");
    assert!(
        output.status.success(),
        "Print Task 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("Claude Task 已启动"));
    assert!(stdout.contains("真实进程输出"));
    assert!(stdout.contains("Claude Task 已完成"));
    let args = fs::read_to_string(fake_bin.join("args")).expect("读取 argv");
    assert!(!args.contains(private_prompt));
    assert!(!args.contains("CLI E2E 完整文档正文"));
    assert!(args.contains("--dangerously-skip-permissions"));
    let stdin = fs::read_to_string(fake_bin.join("stdin")).expect("读取 stdin");
    assert!(stdin.contains(private_prompt));
    assert!(!stdin.contains("CLI E2E 完整文档正文"));
    assert_eq!(
        fs::read_to_string(fake_bin.join("cwd"))
            .expect("读取 cwd")
            .trim(),
        workspace
            .canonicalize()
            .expect("规范化 Workspace")
            .to_string_lossy()
    );
    fs::remove_dir_all(root).expect("清理测试目录");
}
