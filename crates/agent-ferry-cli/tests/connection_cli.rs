use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use agent_ferry_core::AgentFerryPaths;
use agent_ferry_daemon::Daemon;
use agent_ferry_hermes::{
    DevelopmentCredentialStore, HermesConnection, add_connection, remove_connection,
};
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
        // 添加连接会显式验证一次；后续 list/doctor 只读取 daemon 缓存，不能再次访问凭据或远端。
        for _ in 0..1 {
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

fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut expected_length = None;
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).expect("读取 HTTP 请求");
        assert!(read > 0, "HTTP 请求不应提前结束");
        request.extend_from_slice(&buffer[..read]);
        if expected_length.is_none() {
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("Content-Length 合法"))
                    })
                    .unwrap_or(0);
                expected_length = Some(header_end + 4 + content_length);
            }
        }
        if expected_length.is_some_and(|length| request.len() >= length) {
            return request;
        }
    }
}

fn write_http_response(stream: &mut std::net::TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("写入 HTTP 响应");
}

fn spawn_fake_hermes_run() -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("启动 fake Hermes Run");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let task = std::thread::spawn(move || {
        let mut requests = Vec::new();

        let (mut capabilities, _) = listener.accept().expect("接受 capabilities");
        requests.push(read_http_request(&mut capabilities));
        write_http_response(
            &mut capabilities,
            "application/json",
            r#"{"object":"hermes.api_server.capabilities","platform":"hermes-agent","model":"cli-run-e2e","features":{"run_submission":true,"run_status":true,"run_events_sse":true}}"#,
        );

        let (mut submission, _) = listener.accept().expect("接受 run submission");
        requests.push(read_http_request(&mut submission));
        write_http_response(
            &mut submission,
            "application/json",
            r#"{"run_id":"run-cli-e2e","status":"started"}"#,
        );

        let (mut events, _) = listener.accept().expect("接受 SSE");
        requests.push(read_http_request(&mut events));
        write_http_response(
            &mut events,
            "text/event-stream",
            concat!(
                "data: {\"type\":\"run.started\"}\n\n",
                "data: {\"type\":\"tool.started\",\"name\":\"terminal\"}\n\n",
                "data: {\"type\":\"message.delta\",\"delta\":\"处理中\"}\n\n",
                "data: {\"type\":\"tool.completed\",\"name\":\"terminal\"}\n\n",
                "data: {\"type\":\"run.completed\",\"output\":\"完成且已持久化\"}\n\n"
            ),
        );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_submits_input_and_streams_logs_until_terminal() {
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root.clone());
    let (url, server) = spawn_fake_hermes_run();
    let connection = HermesConnection::direct("cli-run-e2e", &url, None).expect("创建 Connection");
    let connection_id = connection.id.clone();
    let store = DevelopmentCredentialStore::new(paths.development_credentials.clone());
    add_connection(&paths, &store, connection, b"cli-run-secret").expect("保存 Connection");

    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));
    let private_input = format!("只应发送给 Hermes 的输入 {}", Uuid::new_v4());
    let mut run = Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", &root)
        .args(["connection", "run", "cli-run-e2e", "--input-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 Hermes Run");
    run.stdin
        .take()
        .expect("获取 stdin")
        .write_all(private_input.as_bytes())
        .expect("写入 input");
    let output = run.wait_with_output().expect("等待 Hermes Run");
    assert!(
        output.status.success(),
        "run 失败: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout 为 UTF-8");
    assert!(stdout.contains("Hermes Run 已提交: run-cli-e2e"));
    assert!(stdout.contains("[tool:start] terminal"));
    assert!(stdout.contains("[output] 处理中"));
    assert!(stdout.contains("[result] 完成且已持久化"));
    assert!(stdout.contains("Hermes Run 已完成"));
    assert!(!stdout.contains(&private_input));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&private_input));

    let requests = server.join().expect("等待 fake Hermes");
    let submission = String::from_utf8_lossy(&requests[1]);
    assert!(submission.starts_with("POST /v1/runs HTTP/1.1"));
    assert!(submission.contains(&private_input));

    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    remove_connection(&paths, &store, &connection_id).expect("清理 Connection");
    fs::remove_dir_all(root).expect("清理测试目录");
}
