use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-opencode-cli-{}", Uuid::new_v4().simple()))
}

fn fake_opencode(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).expect("创建 fake bin");
    let executable = directory.join("opencode-real");
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
case "$1" in
  --version) echo '1.17.18'; exit 0 ;;
  models) echo 'deepseek/deepseek-chat'; exit 0 ;;
  run)
    if [ "$2" = '--help' ]; then
      echo '--format --file --model --dir --auto'
      exit 0
    fi
    pwd > '{0}/cwd'
    printf '%s\n' "$@" > '{0}/args'
    cat > '{0}/stdin'
    previous=''
    for argument in "$@"; do
      if [ "$previous" = '--file' ]; then
        printf '%s' "$argument" > '{0}/artifact-path'
        cp "$argument" '{0}/artifact'
      fi
      previous="$argument"
    done
    if [ "$FAKE_OPENCODE_FAIL" = '1' ]; then
      echo '{{"type":"error","error":{{"data":{{"message":"fake provider rejected"}}}}}}'
      exit 7
    fi
    echo '{{"type":"tool_use","part":{{"tool":"bash","state":{{"status":"completed"}}}}}}'
    echo '{{"type":"text","part":{{"text":"OpenCode 真实进程输出"}}}}'
    echo '{{"type":"step_finish","part":{{"type":"step-finish"}}}}'
    exit 0 ;;
  *) exit 2 ;;
esac
"#,
            directory.display()
        ),
    )
    .expect("写入 fake OpenCode");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    let alias = directory.join("opencode");
    symlink(&executable, &alias).expect("创建 PATH 别名");
    executable
}

fn bind_fake(root: &Path, fake_bin: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .env("PATH", fake_bin)
        .args(["agent", "opencode", "detect", "--json"])
        .output()
        .expect("绑定 fake OpenCode");
    assert!(
        output.status.success(),
        "detect 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"state\": \"ready\""));
    assert!(stdout.contains("deepseek/deepseek-chat"));
    let config = root.join("ferry/config/opencode.json");
    assert_eq!(
        fs::metadata(config)
            .expect("读取绑定权限")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn missing_opencode_only_prints_install_guidance() {
    let root = temporary_root();
    let empty_bin = root.join("empty-bin");
    fs::create_dir_all(&empty_bin).expect("创建空 PATH");
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .env("PATH", empty_bin)
        .args(["agent", "opencode", "detect"])
        .output()
        .expect("运行 OpenCode detect");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("NotDetected"));
    assert!(stdout.contains("https://opencode.ai/docs/"));
    assert!(!root.join("ferry/config/opencode.json").exists());
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn cli_task_uses_explicit_model_file_stdin_and_fixed_workspace() {
    let root = temporary_root();
    let fake_bin = root.join("bin");
    fake_opencode(&fake_bin);
    bind_fake(&root, &fake_bin);

    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("创建 Workspace");
    let document = root.join("document.md");
    fs::write(&document, "OpenCode CLI E2E 完整正文").expect("写入文档");
    let private_prompt = "只允许通过 stdin 发送的 OpenCode Prompt";
    let mut run = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .args([
            "agent",
            "opencode",
            "run",
            "--workspace",
            workspace.to_str().expect("Workspace UTF-8"),
            "--document-file",
            document.to_str().expect("文档路径 UTF-8"),
            "--title",
            "OpenCode CLI E2E",
            "--source-url",
            "https://example.com/opencode-e2e",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 CLI OpenCode Task");
    run.stdin
        .take()
        .expect("获取 stdin")
        .write_all(private_prompt.as_bytes())
        .expect("写入 Prompt");
    let output = run.wait_with_output().expect("等待 OpenCode Task");
    assert!(
        output.status.success(),
        "OpenCode Task 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("OpenCode Task 已启动"));
    assert!(stdout.contains("OpenCode 真实进程输出"));
    assert!(stdout.contains("OpenCode Task 已完成"));
    let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
    assert!(stderr.contains("[opencode tool] bash: completed"));

    let args = fs::read_to_string(fake_bin.join("args")).expect("读取 argv");
    for required in [
        "run",
        "--auto",
        "--format",
        "json",
        "--model",
        "deepseek/deepseek-chat",
        "--file",
        "--dir",
    ] {
        assert!(args.lines().any(|argument| argument == required));
    }
    assert!(!args.contains("--continue"));
    assert!(!args.contains("--session"));
    assert!(!args.contains(private_prompt));
    assert!(!args.contains("OpenCode CLI E2E 完整正文"));
    assert_eq!(
        fs::read_to_string(fake_bin.join("stdin"))
            .expect("读取 stdin")
            .trim(),
        private_prompt
    );
    assert_eq!(
        fs::read_to_string(fake_bin.join("cwd"))
            .expect("读取 cwd")
            .trim(),
        workspace
            .canonicalize()
            .expect("规范化 Workspace")
            .to_string_lossy()
    );
    let artifact = fs::read_to_string(fake_bin.join("artifact")).expect("读取 Artifact 副本");
    assert!(artifact.contains("https://example.com/opencode-e2e"));
    assert!(artifact.ends_with("OpenCode CLI E2E 完整正文"));
    let artifact_path =
        fs::read_to_string(fake_bin.join("artifact-path")).expect("读取 Artifact 路径");
    assert_eq!(
        fs::metadata(artifact_path)
            .expect("读取 Artifact 权限")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn structured_provider_error_returns_nonzero_exit() {
    let root = temporary_root();
    let fake_bin = root.join("bin");
    fake_opencode(&fake_bin);
    bind_fake(&root, &fake_bin);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("创建 Workspace");
    let document = root.join("document.md");
    fs::write(&document, "失败路径正文").expect("写入文档");
    let mut run = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", root.join("ferry"))
        .env("FAKE_OPENCODE_FAIL", "1")
        .args([
            "agent",
            "opencode",
            "run",
            "--workspace",
            workspace.to_str().expect("Workspace UTF-8"),
            "--document-file",
            document.to_str().expect("文档 UTF-8"),
            "--title",
            "失败测试",
            "--source-url",
            "https://example.com/failure",
            "--prompt-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动失败任务");
    run.stdin
        .take()
        .expect("获取 stdin")
        .write_all(b"trigger failure")
        .expect("写入 Prompt");
    let output = run.wait_with_output().expect("等待失败任务");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
    assert!(stderr.contains("fake provider rejected"));
    assert!(stderr.contains("退出码 Some(7)"));
    fs::remove_dir_all(root).expect("清理测试目录");
}
