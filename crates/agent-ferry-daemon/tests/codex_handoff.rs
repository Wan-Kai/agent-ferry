use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_protocol::{
    ConnectorKind, HandoffEvent, HandoffEventKind, HandoffTargetKind, HostResponse, IpcEnvelope,
    ResponseOutcome, read_json_frame, write_json_frame,
};
use serde_json::json;
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-daemon-codex-{}", Uuid::new_v4().simple()))
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
  echo '{{"type":"thread.started","thread_id":"plugin-cli-thread"}}'
  echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"PLUGIN_CODEX_CLI_OK"}}}}'
  echo '{{"type":"turn.completed"}}'
  exit 0
fi
if [ "$1" = "app-server" ]; then
  pwd > '{0}/app-cwd'
  IFS= read -r initialize
  echo '{{"id":0,"result":{{"userAgent":"fake"}}}}'
  IFS= read -r initialized
  IFS= read -r thread_start
  printf '%s\n%s\n%s\n' "$initialize" "$initialized" "$thread_start" > '{0}/app-messages'
  echo '{{"id":1,"result":{{"thread":{{"id":"plugin-app-thread"}}}}}}'
  IFS= read -r turn_start
  printf '%s\n' "$turn_start" >> '{0}/app-messages'
  echo '{{"method":"item/agentMessage/delta","params":{{"threadId":"plugin-app-thread","turnId":"turn-1","itemId":"item-1","delta":"PLUGIN_CODEX_APP_OK"}}}}'
  echo '{{"method":"turn/completed","params":{{"threadId":"plugin-app-thread","turn":{{"id":"turn-1","items":[],"status":"completed"}}}}}}'
  exit 0
fi
exit 1
"#,
            root.display()
        ),
    )
    .expect("写入 fake Codex");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

fn run_handoff(
    paths: &AgentFerryPaths,
    token: &str,
    target_id: &str,
    request_id: &str,
) -> Vec<HandoffEvent> {
    let envelope = IpcEnvelope {
        auth_token: token.to_owned(),
        connector: ConnectorKind::ChromeNativeHost,
        request: json!({
            "protocol_version": 1,
            "request_id": request_id,
            "command": {
                "type": "handoff",
                "task_id": format!("task-{}", Uuid::new_v4().simple()),
                "target_id": target_id,
                "prompt": "只交给所选 Codex Target 的 Prompt",
                "source": {
                    "url": "https://example.com/codex-article",
                    "title": "Codex 插件联调文章",
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
    write_json_frame(&mut stream, &envelope).expect("发送 Codex Handoff");
    stream.flush().expect("刷新请求");
    let mut events = Vec::new();
    loop {
        let event: HandoffEvent = read_json_frame(&mut stream).expect("读取 Codex 事件");
        let terminal = matches!(
            event.event,
            HandoffEventKind::Completed | HandoffEventKind::Failed | HandoffEventKind::Cancelled
        );
        events.push(event);
        if terminal {
            return events;
        }
    }
}

fn assert_successful_events(events: &[HandoffEvent], expected: &str) {
    assert_eq!(
        events.last().map(|event| event.event),
        Some(HandoffEventKind::Completed)
    );
    assert!(events.iter().any(|event| {
        event.event == HandoffEventKind::OutputDelta && event.text.as_deref() == Some(expected)
    }));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].sequence == pair[0].sequence + 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chrome_handoff_routes_codex_cli_and_app_to_separate_runners() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.join("ferry"));
    let fake_root = root.join("fake");
    let executable = fake_codex(&fake_root);
    agent_ferry_codex::bind(&paths, &executable).expect("绑定 fake Codex");
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

    let status_envelope = IpcEnvelope {
        auth_token: token.clone(),
        connector: ConnectorKind::ChromeNativeHost,
        request: json!({
            "protocol_version": 1,
            "request_id": "codex-status-e2e",
            "command": {"type": "status"}
        }),
    };
    let mut status_stream = UnixStream::connect(&paths.socket).expect("连接 Status socket");
    write_json_frame(&mut status_stream, &status_envelope).expect("发送 Status");
    let status: HostResponse = read_json_frame(&mut status_stream).expect("读取 Status");
    let ResponseOutcome::Success { result } = status.outcome else {
        panic!("Status 应成功");
    };
    assert!(result.targets.iter().any(|target| {
        target.id == format!("codex-cli-{}", workspace.id)
            && target.kind == HandoffTargetKind::LocalCodexCli
    }));
    assert!(result.targets.iter().any(|target| {
        target.id == format!("codex-app-{}", workspace.id)
            && target.kind == HandoffTargetKind::LocalCodexApp
    }));

    let cli_events = run_handoff(
        &paths,
        &token,
        &format!("codex-cli-{}", workspace.id),
        "codex-cli-plugin-e2e",
    );
    assert_successful_events(&cli_events, "PLUGIN_CODEX_CLI_OK");
    let app_events = run_handoff(
        &paths,
        &token,
        &format!("codex-app-{}", workspace.id),
        "codex-app-plugin-e2e",
    );
    assert_successful_events(&app_events, "PLUGIN_CODEX_APP_OK");

    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");

    let canonical_workspace = workspace_path
        .canonicalize()
        .expect("规范化 Workspace")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        fs::read_to_string(fake_root.join("cli-cwd"))
            .expect("读取 CLI cwd")
            .trim(),
        canonical_workspace
    );
    assert_eq!(
        fs::read_to_string(fake_root.join("app-cwd"))
            .expect("读取 App cwd")
            .trim(),
        canonical_workspace
    );
    let cli_args = fs::read_to_string(fake_root.join("cli-args")).expect("读取 CLI argv");
    assert!(!cli_args.contains("只交给所选"));
    let cli_stdin = fs::read_to_string(fake_root.join("cli-stdin")).expect("读取 CLI stdin");
    assert!(cli_stdin.contains("只交给所选 Codex Target 的 Prompt"));
    let app_messages = fs::read_to_string(fake_root.join("app-messages")).expect("读取 App 消息");
    assert!(app_messages.contains("thread/start"));
    assert!(app_messages.contains("turn/start"));
    assert!(app_messages.contains("只交给所选 Codex Target 的 Prompt"));
    fs::remove_dir_all(root).expect("清理测试目录");
}
