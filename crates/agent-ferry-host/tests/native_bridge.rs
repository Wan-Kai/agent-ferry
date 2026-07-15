use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use agent_ferry_core::AgentFerryPaths;
use agent_ferry_daemon::Daemon;
use agent_ferry_protocol::{
    Command as HostCommand, HostRequest, HostResponse, PROTOCOL_VERSION, ResponseOutcome,
    read_json_frame, write_json_frame,
};
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-h-{}", Uuid::new_v4().simple()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_host_process_bridges_chrome_frame_to_daemon_and_logs_to_stderr() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.clone());
    let daemon = Daemon::bind(paths).expect("绑定 daemon");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));

    let mut child = Command::new(env!("CARGO_BIN_EXE_agentferry-host"))
        .env("AGENT_FERRY_HOME", &root)
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 Native Host");
    let request = HostRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: "chrome-e2e".to_owned(),
        command: HostCommand::Status,
    };
    let mut stdin = child.stdin.take().expect("获取 Native Host stdin");
    write_json_frame(&mut stdin, &request).expect("模拟 Chrome 写入消息");
    stdin.flush().expect("刷新 stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("等待 Native Host");
    assert!(output.status.success());
    let response: HostResponse =
        read_json_frame(&mut output.stdout.as_slice()).expect("读取 Chrome 响应");
    match response.outcome {
        ResponseOutcome::Success { result } => {
            assert_eq!(result.daemon, agent_ferry_protocol::ServiceState::Ready);
            assert_eq!(
                result.chrome_extension,
                agent_ferry_protocol::ServiceState::Ready
            );
        }
        ResponseOutcome::Failure { error } => panic!("桥接失败: {error:?}"),
    }
    let logs = String::from_utf8(output.stderr).expect("stderr 应为 UTF-8");
    assert!(logs.contains("Native Host 已启动"), "实际日志: {logs}");
    assert!(logs.contains("Native Host 已退出"), "实际日志: {logs}");

    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    fs::remove_dir_all(root).expect("清理测试目录");
}
