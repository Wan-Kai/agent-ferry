use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_protocol::{
    ConnectorKind, HandoffEvent, HandoffEventKind, IpcEnvelope, read_json_frame, write_json_frame,
};
use serde_json::json;
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-daemon-claude-{}", Uuid::new_v4().simple()))
}

fn fake_claude(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("创建 fake Claude 目录");
    let executable = root.join("claude");
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
case "$1" in
  --version) echo '2.1.197 (Claude Code)'; exit 0 ;;
  --help) echo '--print --output-format --permission-mode --dangerously-skip-permissions'; exit 0 ;;
  auth) echo '{{"loggedIn":true}}'; exit 0 ;;
esac
pwd > '{0}/cwd'
printf '%s\n' "$@" > '{0}/args'
cat > '{0}/stdin'
echo '{{"type":"stream_event","event":{{"delta":{{"text":"PLUGIN_CLAUDE_E2E_OK"}}}}}}'
echo '{{"type":"result","result":"PLUGIN_CLAUDE_E2E_OK"}}'
"#,
            root.display()
        ),
    )
    .expect("写入 fake Claude");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chrome_handoff_runs_claude_in_configured_workspace() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.join("ferry"));
    let fake_root = root.join("fake");
    let executable = fake_claude(&fake_root);
    agent_ferry_claude::bind(&paths, &executable).expect("绑定 fake Claude");
    let workspace_path = root.join("workspace");
    fs::create_dir_all(&workspace_path).expect("创建 Workspace");
    let workspace = agent_ferry_core::workspace::add(&paths, "test-workspace", &workspace_path)
        .expect("保存 Workspace");

    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let token = load_connector_token(&paths).expect("读取 Connector token");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));

    let task_id = format!("task-{}", Uuid::new_v4().simple());
    let envelope = IpcEnvelope {
        auth_token: token,
        connector: ConnectorKind::ChromeNativeHost,
        request: json!({
            "protocol_version": 1,
            "request_id": "claude-plugin-e2e",
            "command": {
                "type": "handoff",
                "task_id": task_id,
                "target_id": format!("claude-{}", workspace.id),
                "prompt": "只通过 stdin 进入 Claude Code 的 Prompt",
                "source": {
                    "url": "https://example.com/claude-article",
                    "title": "Claude Code 插件联调文章",
                    "author": "测试作者",
                    "published": "2026-07-17",
                    "site": "示例站点",
                    "extractor": "defuddle",
                    "markdown": format!("# 正文\n\n{}", "完整页面内容。".repeat(100)),
                    "word_count": 700
                }
            }
        }),
    };
    let mut stream = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope).expect("发送 Claude Handoff");
    stream.flush().expect("刷新请求");

    let mut events = Vec::new();
    loop {
        let event: HandoffEvent = read_json_frame(&mut stream).expect("读取 Claude 事件");
        let terminal = matches!(
            event.event,
            HandoffEventKind::Completed | HandoffEventKind::Failed | HandoffEventKind::Cancelled
        );
        events.push(event);
        if terminal {
            break;
        }
    }
    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");

    assert_eq!(
        events.last().map(|event| event.event),
        Some(HandoffEventKind::Completed)
    );
    assert!(events.iter().any(|event| {
        event.event == HandoffEventKind::OutputDelta
            && event.text.as_deref() == Some("PLUGIN_CLAUDE_E2E_OK")
    }));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].sequence == pair[0].sequence + 1)
    );
    assert_eq!(
        fs::read_to_string(fake_root.join("cwd"))
            .expect("读取 cwd")
            .trim(),
        workspace_path
            .canonicalize()
            .expect("规范化 Workspace")
            .to_string_lossy()
    );
    let args = fs::read_to_string(fake_root.join("args")).expect("读取 argv");
    assert!(args.contains("--dangerously-skip-permissions"));
    assert!(!args.contains("只通过 stdin"));
    let stdin = fs::read_to_string(fake_root.join("stdin")).expect("读取 stdin");
    assert!(stdin.contains("只通过 stdin 进入 Claude Code 的 Prompt"));
    assert!(stdin.contains("完整来源内容位于以下只读交接 Artifact"));
    fs::remove_dir_all(root).expect("清理测试目录");
}
