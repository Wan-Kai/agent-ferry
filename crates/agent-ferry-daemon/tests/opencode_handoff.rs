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
    PathBuf::from(format!(
        "/tmp/af-daemon-opencode-{}",
        Uuid::new_v4().simple()
    ))
}

fn fake_opencode(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("创建 fake OpenCode 目录");
    let executable = root.join("opencode");
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
case "$1" in
  --version) echo '1.17.18'; exit 0 ;;
  models) echo 'deepseek/deepseek-chat'; exit 0 ;;
  run)
    if [ "$2" = '--help' ]; then echo '--format --file --model --dir --auto'; exit 0; fi
    pwd > '{0}/cwd'
    printf '%s\n' "$@" > '{0}/args'
    cat > '{0}/stdin'
    echo '{{"type":"tool_use","part":{{"tool":"read","state":{{"status":"completed"}}}}}}'
    echo '{{"type":"text","part":{{"text":"PLUGIN_OPENCODE_E2E_OK"}}}}'
    exit 0 ;;
esac
exit 2
"#,
            root.display()
        ),
    )
    .expect("写入 fake OpenCode");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("设置执行权限");
    executable
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chrome_handoff_runs_opencode_in_configured_workspace() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.join("ferry"));
    let fake_root = root.join("fake");
    let executable = fake_opencode(&fake_root);
    agent_ferry_opencode::bind(&paths, &executable, "deepseek/deepseek-chat")
        .expect("绑定 fake OpenCode");
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
    let request = json!({
        "protocol_version": 1,
        "request_id": "opencode-plugin-e2e",
        "command": {
            "type": "handoff",
            "task_id": task_id,
            "target_id": format!("opencode-{}", workspace.id),
            "prompt": "只通过 stdin 进入 OpenCode 的 Prompt",
            "source": {
                "url": "https://example.com/opencode-article",
                "title": "OpenCode 插件联调文章",
                "author": "测试作者",
                "published": "2026-07-17",
                "site": "示例站点",
                "extractor": "defuddle",
                "markdown": format!("# 正文\n\n{}", "完整页面内容。".repeat(100)),
                "word_count": 700
            }
        }
    });
    let envelope = IpcEnvelope {
        auth_token: token,
        connector: ConnectorKind::ChromeNativeHost,
        request,
    };
    let mut stream = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope).expect("发送 OpenCode Handoff");
    stream.flush().expect("刷新请求");

    let mut events = Vec::new();
    loop {
        let event: HandoffEvent = read_json_frame(&mut stream).expect("读取 OpenCode 事件");
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
            && event.text.as_deref() == Some("PLUGIN_OPENCODE_E2E_OK")
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
    assert!(args.lines().any(|line| line == "deepseek/deepseek-chat"));
    assert!(!args.contains("只通过 stdin"));
    let stdin = fs::read_to_string(fake_root.join("stdin")).expect("读取 stdin");
    assert_eq!(stdin, "只通过 stdin 进入 OpenCode 的 Prompt");
    fs::remove_dir_all(root).expect("清理测试目录");
}
