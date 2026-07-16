use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_protocol::{
    ConnectorKind, ErrorCode, HostResponse, IpcEnvelope, ResponseOutcome, read_json_frame,
    write_json_frame,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-d-{}", Uuid::new_v4().simple()))
}

fn send_envelope(paths: &AgentFerryPaths, auth_token: String, request: Value) -> HostResponse {
    send_as(paths, auth_token, ConnectorKind::Cli, request)
}

fn send_as(
    paths: &AgentFerryPaths,
    auth_token: String,
    connector: ConnectorKind,
    request: Value,
) -> HostResponse {
    let envelope = IpcEnvelope {
        auth_token,
        connector,
        request,
    };
    let mut stream = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope).expect("写入请求");
    read_json_frame(&mut stream).expect("读取响应")
}

fn failure_code(response: HostResponse) -> ErrorCode {
    match response.outcome {
        ResponseOutcome::Failure { error } => error.code,
        ResponseOutcome::Success { .. } => panic!("预期失败响应"),
    }
}

fn assert_chrome_cannot_admin_or_run(paths: &AgentFerryPaths, token: &str) {
    let extension_admin = send_as(
        paths,
        token.to_owned(),
        ConnectorKind::ChromeNativeHost,
        json!({
            "protocol_version": 1,
            "request_id": "extension-admin",
            "command": {
                "type": "connection_add",
                "name": "forbidden",
                "base_url": "http://127.0.0.1:8642",
                "model": null,
                "token": "must-not-be-stored"
            }
        }),
    );
    assert_eq!(failure_code(extension_admin), ErrorCode::PermissionDenied);
    assert!(!paths.hermes_connections.exists());

    let extension_direct_run = send_as(
        paths,
        token.to_owned(),
        ConnectorKind::ChromeNativeHost,
        json!({
            "protocol_version": 1,
            "request_id": "extension-direct-run",
            "command": {
                "type": "hermes_run",
                "task_id": "task-1",
                "target_id": "remote-1",
                "input": "浏览器不能绕过页面交接边界直接执行任意 input"
            }
        }),
    );
    assert_eq!(
        failure_code(extension_direct_run),
        ErrorCode::PermissionDenied
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn socket_boundary_rejects_unauthorized_version_unknown_and_malformed_requests() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.clone());
    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let token = load_connector_token(&paths).expect("读取 token");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));

    let valid = send_envelope(
        &paths,
        token.clone(),
        json!({
            "protocol_version": 1,
            "request_id": "valid",
            "command": { "type": "status" }
        }),
    );
    assert!(matches!(valid.outcome, ResponseOutcome::Success { .. }));

    let unauthorized = send_envelope(
        &paths,
        "not-the-token".to_owned(),
        json!({
            "protocol_version": 1,
            "request_id": "unauthorized",
            "command": { "type": "status" }
        }),
    );
    assert_eq!(failure_code(unauthorized), ErrorCode::AuthenticationFailed);

    let version_mismatch = send_envelope(
        &paths,
        token.clone(),
        json!({
            "protocol_version": 999,
            "request_id": "version-mismatch",
            "command": { "type": "status" }
        }),
    );
    assert_eq!(
        failure_code(version_mismatch),
        ErrorCode::ProtocolVersionUnsupported
    );

    assert_chrome_cannot_admin_or_run(&paths, &token);

    let unknown_command = send_envelope(
        &paths,
        token,
        json!({
            "protocol_version": 1,
            "request_id": "unknown-command",
            "command": { "type": "delete_everything" }
        }),
    );
    assert_eq!(failure_code(unknown_command), ErrorCode::UnknownCommand);

    let mut malformed = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    malformed
        .write_all(&(5_u32).to_le_bytes())
        .expect("写入 framing");
    malformed.write_all(b"nope!").expect("写入畸形 JSON");
    let malformed_response: HostResponse = read_json_frame(&mut malformed).expect("读取错误响应");
    assert_eq!(failure_code(malformed_response), ErrorCode::InvalidMessage);

    let mut disconnected = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    disconnected
        .write_all(&(100_u32).to_le_bytes())
        .expect("写入不完整 framing");
    drop(disconnected);

    let mut still_alive = UnixStream::connect(&paths.socket).expect("daemon 应继续监听");
    still_alive
        .write_all(&(2_u32).to_le_bytes())
        .expect("写入探测消息");
    still_alive.write_all(b"{}").expect("写入探测正文");
    let mut length = [0_u8; 4];
    still_alive
        .read_exact(&mut length)
        .expect("daemon 应返回响应");

    shutdown_tx.send(()).expect("停止 daemon");
    task.await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    fs::remove_dir_all(root).expect("清理测试目录");
}
