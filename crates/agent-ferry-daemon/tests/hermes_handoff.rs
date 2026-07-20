use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_hermes::{
    CredentialStore, DevelopmentCredentialStore, HermesConnection, add_connection,
    remove_connection,
};
use agent_ferry_protocol::{
    ConnectorKind, HandoffEvent, HandoffEventKind, HostResponse, IpcEnvelope, ResponseOutcome,
    read_json_frame, write_json_frame,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-handoff-{}", Uuid::new_v4().simple()))
}

struct ConnectionCleanup {
    paths: AgentFerryPaths,
    connection_id: String,
}

impl Drop for ConnectionCleanup {
    fn drop(&mut self) {
        let store = DevelopmentCredentialStore::new(self.paths.development_credentials.clone());
        let _ = remove_connection(&self.paths, &store, &self.connection_id);
        let _ = fs::remove_dir_all(&self.paths.root);
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_length = None;
    loop {
        let read = stream.read(&mut buffer).await.expect("读取 Hermes 请求");
        assert!(read > 0, "Hermes 请求不应提前结束");
        request.extend_from_slice(&buffer[..read]);
        if expected_length.is_none() {
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| {
                            value
                                .trim()
                                .parse::<usize>()
                                .expect("Content-Length 应合法")
                        })
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

async fn write_http_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("写入 Hermes 响应");
}

async fn fake_hermes() -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();

        let (mut capabilities, _) = listener.accept().await.expect("接受 capabilities 请求");
        requests.push(read_http_request(&mut capabilities).await);
        write_http_response(
            &mut capabilities,
            "application/json",
            r#"{"object":"hermes.api_server.capabilities","platform":"hermes-agent","model":"e2e","features":{"run_submission":true,"run_status":true,"run_events_sse":true}}"#,
        )
        .await;

        let (mut submission, _) = listener.accept().await.expect("接受 run 提交请求");
        requests.push(read_http_request(&mut submission).await);
        write_http_response(
            &mut submission,
            "application/json",
            r#"{"run_id":"run-daemon-e2e","status":"started"}"#,
        )
        .await;

        let (mut events, _) = listener.accept().await.expect("接受 SSE 请求");
        requests.push(read_http_request(&mut events).await);
        write_http_response(
            &mut events,
            "text/event-stream",
            concat!(
                "data: {\"type\":\"run.started\"}\n\n",
                "data: {\"type\":\"message.delta\",\"delta\":\"正在分析\"}\n\n",
                "data: {\"type\":\"run.completed\",\"output\":\"分析完成\"}\n\n"
            ),
        )
        .await;

        requests
    });
    (format!("http://{address}"), task)
}

fn request_body(request: &[u8]) -> Value {
    let body_start = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("HTTP 请求包含 header 边界")
        + 4;
    serde_json::from_slice(&request[body_start..]).expect("Run body 应为 JSON")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn chrome_handoff_streams_ordered_events_and_submits_complete_page() {
    let (base_url, fake_task) = fake_hermes().await;
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root);
    let connection = HermesConnection::direct("daemon-e2e", &base_url, None)
        .expect("创建临时 Hermes Connection");
    let connection_id = connection.id.clone();
    let credential_ref = connection.credential_ref.clone();
    let store = DevelopmentCredentialStore::new(paths.development_credentials.clone());
    add_connection(&paths, &store, connection, b"daemon-e2e-secret")
        .expect("保存临时 Connection 与开发凭据");
    let cleanup = ConnectionCleanup {
        paths: paths.clone(),
        connection_id: connection_id.clone(),
    };

    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let token = load_connector_token(&paths).expect("读取 Connector token");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));

    let task_id = format!("task-{}", Uuid::new_v4().simple());
    let markdown = format!(
        "# 完整正文\n\n{}",
        "这是不可截断的页面正文段落。".repeat(80)
    );
    let request = json!({
        "protocol_version": 1,
        "request_id": "handoff-daemon-e2e",
        "command": {
            "type": "handoff",
            "task_id": task_id,
            "target_id": connection_id,
            "prompt": "请逐段分析这篇文章",
            "source": {
                "url": "https://example.com/full-article",
                "title": "完整文章",
                "author": "测试作者",
                "published": "2026-07-16",
                "site": "示例站点",
                "extractor": "defuddle",
                "markdown": markdown,
                "word_count": 800
            }
        }
    });
    let envelope = IpcEnvelope {
        auth_token: token,
        connector: ConnectorKind::ChromeNativeHost,
        request,
    };
    let mut stream = UnixStream::connect(&paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope).expect("发送 Handoff");

    let mut events = Vec::new();
    loop {
        let event: HandoffEvent = read_json_frame(&mut stream).expect("读取 HandoffEvent");
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
    let requests = fake_task.await.expect("等待 fake Hermes");

    assert!(events.len() >= 3, "应包含提交、运行和终态事件");
    assert!(events.iter().all(|event| event.task_id == task_id));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].sequence == pair[0].sequence + 1),
        "sequence 必须严格逐一递增"
    );
    assert_eq!(
        events.last().map(|event| event.event),
        Some(HandoffEventKind::Completed)
    );

    let capabilities_request = String::from_utf8_lossy(&requests[0]);
    let submission_request = String::from_utf8_lossy(&requests[1]);
    let events_request = String::from_utf8_lossy(&requests[2]);
    assert!(capabilities_request.starts_with("GET /v1/capabilities HTTP/1.1"));
    assert!(submission_request.starts_with("POST /v1/runs HTTP/1.1"));
    assert!(events_request.starts_with("GET /v1/runs/run-daemon-e2e/events HTTP/1.1"));
    assert!(requests.iter().all(|request| {
        String::from_utf8_lossy(request).contains("authorization: Bearer daemon-e2e-secret")
    }));
    let body = request_body(&requests[1]);
    let input = body["input"].as_str().expect("提交体包含 input");
    assert!(input.contains("请逐段分析这篇文章"));
    assert!(input.contains("来源 URL: https://example.com/full-article"));
    assert!(
        input.ends_with(&markdown),
        "完整 Markdown 必须位于 input 尾部"
    );

    drop(cleanup);
    assert!(
        DevelopmentCredentialStore::new(paths.development_credentials.clone())
            .get(&credential_ref)
            .expect("检查临时开发凭据")
            .is_none(),
        "测试结束后必须删除临时开发凭据"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_uses_configured_targets_without_contacting_hermes() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动被动 Hermes 监听器");
    let base_url = format!("http://{}", listener.local_addr().expect("读取监听地址"));
    let root = temporary_root();
    let paths = AgentFerryPaths::from_root(root);
    let connection = HermesConnection::direct("passive-status", &base_url, None)
        .expect("创建临时 Hermes Connection");
    let connection_id = connection.id.clone();
    let store = DevelopmentCredentialStore::new(paths.development_credentials.clone());
    add_connection(&paths, &store, connection, b"passive-status-secret")
        .expect("保存临时 Connection");
    let cleanup = ConnectionCleanup {
        paths: paths.clone(),
        connection_id,
    };

    let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
    let token = load_connector_token(&paths).expect("读取 Connector token");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let daemon_task = tokio::spawn(daemon.serve_until(async {
        let _ = shutdown_rx.await;
    }));
    let socket = paths.socket.clone();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            let envelope = IpcEnvelope {
                auth_token: token,
                connector: ConnectorKind::ChromeNativeHost,
                request: json!({
                    "protocol_version": 1,
                    "request_id": "passive-status",
                    "command": { "type": "status" }
                }),
            };
            let mut stream = UnixStream::connect(socket).expect("连接 daemon socket");
            write_json_frame(&mut stream, &envelope).expect("发送 Status");
            read_json_frame::<_, HostResponse>(&mut stream).expect("读取 Status")
        }),
    )
    .await
    .expect("Status 不应等待远程 Hermes")
    .expect("等待 Status 线程");
    assert!(
        matches!(response.outcome, ResponseOutcome::Success { .. }),
        "Status 应返回缓存配置"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
            .await
            .is_err(),
        "Status 不应连接远程 Hermes"
    );

    shutdown_tx.send(()).expect("停止 daemon");
    daemon_task
        .await
        .expect("等待 daemon task")
        .expect("daemon 正常退出");
    drop(cleanup);
}
