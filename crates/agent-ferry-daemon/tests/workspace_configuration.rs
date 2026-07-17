use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_protocol::{
    ConnectorKind, HostResponse, IpcEnvelope, ResponseOutcome, read_json_frame, write_json_frame,
};
use serde_json::json;
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/af-workspace-config-{}",
        Uuid::new_v4().simple()
    ))
}

fn send(paths: &AgentFerryPaths, token: &str, command: &serde_json::Value) -> HostResponse {
    let envelope = IpcEnvelope {
        auth_token: token.to_owned(),
        connector: ConnectorKind::ChromeNativeHost,
        request: json!({
            "protocol_version": 1,
            "request_id": Uuid::new_v4().to_string(),
            "command": command,
        }),
    };
    let mut stream = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope).expect("发送 Workspace 命令");
    read_json_frame(&mut stream).expect("读取 Workspace 响应")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chrome_can_add_and_remove_existing_workspace_without_touching_directory() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.join("ferry"));
    let workspace_path = root.join("user-project");
    fs::create_dir_all(&workspace_path).expect("创建用户目录");
    fs::write(workspace_path.join("keep.txt"), "不能删除").expect("写入用户文件");
    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let token = load_connector_token(&paths).expect("读取 Connector token");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));

    let add = send(
        &paths,
        &token,
        &json!({
            "type": "workspace_add",
            "name": "user-project",
            "path": workspace_path,
        }),
    );
    let workspace_id = match add.outcome {
        ResponseOutcome::Success { result } => {
            assert!(
                result
                    .capabilities
                    .iter()
                    .any(|item| item == "workspace.write")
            );
            let workspace = result.workspaces.first().expect("返回新增 Workspace");
            assert_eq!(workspace.name, "user-project");
            assert!(workspace.ready);
            workspace.id.clone()
        }
        ResponseOutcome::Failure { error } => panic!("新增 Workspace 失败: {error:?}"),
    };

    let remove = send(
        &paths,
        &token,
        &json!({ "type": "workspace_remove", "identifier": workspace_id }),
    );
    match remove.outcome {
        ResponseOutcome::Success { result } => assert!(result.workspaces.is_empty()),
        ResponseOutcome::Failure { error } => panic!("移除 Workspace 失败: {error:?}"),
    }
    assert_eq!(
        fs::read_to_string(workspace_path.join("keep.txt")).expect("用户文件仍存在"),
        "不能删除"
    );

    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    fs::remove_dir_all(root).expect("清理测试目录");
}
