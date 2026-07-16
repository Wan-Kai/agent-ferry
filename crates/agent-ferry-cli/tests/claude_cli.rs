use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

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
