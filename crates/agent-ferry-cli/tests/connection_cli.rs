use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use agent_ferry_core::AgentFerryPaths;
use agent_ferry_daemon::Daemon;
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-c-{}", Uuid::new_v4().simple()))
}

fn spawn_fake_hermes() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let task = std::thread::spawn(move || {
        let body = r#"{
            "object":"hermes.api_server.capabilities",
            "platform":"hermes-agent",
            "model":"cli-e2e",
            "features":{
                "run_submission":true,
                "run_status":true,
                "run_events_sse":true
            }
        }"#;
        let mut requests = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("接受诊断请求");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).expect("读取 HTTP 请求");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("写入 HTTP 响应");
            requests.push_str(&String::from_utf8(request).expect("请求应为 UTF-8"));
        }
        requests
    });
    (format!("http://{address}"), task)
}

#[test]
fn setup_rejects_invalid_name_before_daemon_or_remote_changes() {
    let root = temporary_root();
    let output = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args([
            "connection",
            "setup",
            "hermes",
            "--name",
            " invalid ",
            "--ssh-host",
            "must-not-connect",
            "--yes",
        ])
        .output()
        .expect("运行非法 setup");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("setup stderr");
    assert!(stderr.contains("Connection 名称"));
    assert!(!stderr.contains("agentferryd"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_add_list_and_doctor_keep_token_out_of_files_and_output() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.clone());
    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));
    let (url, server) = spawn_fake_hermes();
    let token = format!("cli-secret-{}", Uuid::new_v4());

    let mut add = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args([
            "connection",
            "add",
            "hermes",
            "--name",
            "cli-e2e",
            "--url",
            &url,
            "--token-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 connection add");
    add.stdin
        .take()
        .expect("获取 stdin")
        .write_all(token.as_bytes())
        .expect("写入 token");
    let add_output = add.wait_with_output().expect("等待 connection add");
    assert!(add_output.status.success());
    assert!(!String::from_utf8_lossy(&add_output.stdout).contains(&token));
    assert!(!String::from_utf8_lossy(&add_output.stderr).contains(&token));

    let config = fs::read_to_string(&paths.hermes_connections).expect("读取配置");
    assert!(!config.contains(&token));
    let mode = fs::metadata(&paths.hermes_connections)
        .expect("读取配置权限")
        .permissions();
    assert_eq!(mode.mode() & 0o777, 0o600);

    let list = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args(["connection", "list", "--json"])
        .output()
        .expect("运行 connection list");
    assert!(list.status.success());
    let list_text = String::from_utf8(list.stdout).expect("list JSON");
    assert!(list_text.contains("cli-e2e"));
    assert!(!list_text.contains(&token));

    let doctor = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args(["connection", "doctor", "cli-e2e", "--json"])
        .output()
        .expect("运行 connection doctor");
    assert!(
        doctor.status.success(),
        "doctor 失败: stdout={} stderr={}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_text = String::from_utf8(doctor.stdout).expect("doctor JSON");
    assert!(doctor_text.contains("\"state\": \"ready\""));
    assert!(doctor_text.contains("run.events_sse"));
    assert!(!doctor_text.contains(&token));

    let request = server.join().expect("等待 fake Hermes");
    assert!(request.contains(&format!("authorization: Bearer {token}")));

    let remove = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args(["connection", "remove", "cli-e2e"])
        .output()
        .expect("运行 connection remove");
    assert!(
        remove.status.success(),
        "删除失败: {}",
        String::from_utf8_lossy(&remove.stderr)
    );
    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    fs::remove_dir_all(root).expect("清理测试目录");
}
