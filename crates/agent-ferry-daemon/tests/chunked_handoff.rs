use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_ferry_core::{AgentFerryPaths, load_connector_token};
use agent_ferry_daemon::Daemon;
use agent_ferry_hermes::{
    DevelopmentCredentialStore, HermesConnection, add_connection, remove_connection,
};
use agent_ferry_protocol::{
    ConnectorKind, ErrorCode, HandoffEvent, HandoffEventKind, HandoffTransferAck,
    HandoffTransferPhase, HostResponse, IpcEnvelope, MAX_HANDOFF_CHUNK_BYTES, MAX_HANDOFF_CHUNKS,
    MAX_HANDOFF_CONTENT_BYTES, ResponseOutcome, read_json_frame, write_json_frame,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/af-chunked-handoff-{}",
        Uuid::new_v4().simple()
    ))
}

struct TestDaemon {
    paths: AgentFerryPaths,
    token: String,
    connection_id: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl TestDaemon {
    fn start(base_url: &str) -> Self {
        let paths = AgentFerryPaths::from_root(temporary_root());
        let connection = HermesConnection::direct("chunked-e2e", base_url, None)
            .expect("创建临时 Hermes Connection");
        let connection_id = connection.id.clone();
        let store = DevelopmentCredentialStore::new(paths.development_credentials.clone());
        add_connection(&paths, &store, connection, b"chunked-e2e-secret")
            .expect("保存临时 Connection 与开发凭据");

        let daemon = Daemon::bind(paths.clone()).expect("绑定 daemon");
        let token = load_connector_token(&paths).expect("读取 Connector token");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(daemon.serve_until(async {
            let _ = shutdown_rx.await;
        }));
        Self {
            paths,
            token,
            connection_id,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        }
    }

    async fn stop(mut self) {
        self.shutdown
            .take()
            .expect("shutdown sender 存在")
            .send(())
            .expect("停止 daemon");
        self.task
            .take()
            .expect("daemon task 存在")
            .await
            .expect("等待 daemon task")
            .expect("daemon 正常退出");
        let store = DevelopmentCredentialStore::new(self.paths.development_credentials.clone());
        remove_connection(&self.paths, &store, &self.connection_id)
            .expect("删除临时 Hermes Connection");
        fs::remove_dir_all(&self.paths.root).expect("清理测试目录");
    }
}

fn envelope(daemon: &TestDaemon, request: Value) -> IpcEnvelope {
    IpcEnvelope {
        auth_token: daemon.token.clone(),
        connector: ConnectorKind::ChromeNativeHost,
        request,
    }
}

fn send_value(daemon: &TestDaemon, request: Value) -> Value {
    let mut stream = UnixStream::connect(&daemon.paths.socket).expect("连接 daemon socket");
    write_json_frame(&mut stream, &envelope(daemon, request)).expect("发送 IPC 请求");
    read_json_frame(&mut stream).expect("读取 IPC 响应")
}

fn begin_request(
    daemon: &TestDaemon,
    request_id: &str,
    task_id: &str,
    body: &[u8],
    total_chunks: u32,
    sha256: &str,
) -> Value {
    json!({
        "protocol_version": 1,
        "request_id": request_id,
        "command": {
            "type": "handoff_begin",
            "task_id": task_id,
            "target_id": daemon.connection_id,
            "prompt": format!("请分析 {task_id}"),
            "source": {
                "url": format!("https://example.com/{task_id}"),
                "title": format!("正文 {task_id}"),
                "author": "测试作者",
                "published": "2026-07-16",
                "site": "示例站点",
                "extractor": "defuddle",
                "word_count": 800
            },
            "total_bytes": body.len(),
            "total_chunks": total_chunks,
            "sha256": sha256
        }
    })
}

fn valid_begin_request(
    daemon: &TestDaemon,
    request_id: &str,
    task_id: &str,
    body: &[u8],
    total_chunks: u32,
) -> Value {
    begin_request(
        daemon,
        request_id,
        task_id,
        body,
        total_chunks,
        &format!("{:x}", Sha256::digest(body)),
    )
}

fn chunk_request(request_id: &str, task_id: &str, index: u32, data: &str) -> Value {
    json!({
        "protocol_version": 1,
        "request_id": request_id,
        "command": {
            "type": "handoff_chunk",
            "task_id": task_id,
            "index": index,
            "data": data
        }
    })
}

fn end_request(request_id: &str, task_id: &str) -> Value {
    json!({
        "protocol_version": 1,
        "request_id": request_id,
        "command": { "type": "handoff_end", "task_id": task_id }
    })
}

fn assert_ack(value: Value, phase: HandoffTransferPhase, next_index: u32) {
    let ack: HandoffTransferAck = serde_json::from_value(value).expect("响应应为传输 ACK");
    assert_eq!(ack.phase, phase);
    assert_eq!(ack.next_index, next_index);
}

fn failure(value: Value) -> (ErrorCode, String) {
    let response: HostResponse = serde_json::from_value(value).expect("响应应为协议错误");
    match response.outcome {
        ResponseOutcome::Failure { error } => (error.code, error.message),
        ResponseOutcome::Success { .. } => panic!("预期失败响应"),
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 8192];
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

fn http_body(request: &[u8]) -> Value {
    let body_start = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("HTTP 请求包含 header 边界")
        + 4;
    serde_json::from_slice(&request[body_start..]).expect("HTTP body 应为 JSON")
}

async fn fake_hermes(expected_requests: usize) -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().await.expect("接受 Hermes 请求");
            let request = read_http_request(&mut stream).await;
            let first_line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .expect("HTTP 请求行存在")
                .to_owned();
            if first_line == "GET /v1/capabilities HTTP/1.1" {
                write_http_response(
                    &mut stream,
                    "application/json",
                    r#"{"object":"hermes.api_server.capabilities","platform":"hermes-agent","model":"e2e","features":{"run_submission":true,"run_status":true,"run_events_sse":true}}"#,
                )
                .await;
            } else if first_line == "POST /v1/runs HTTP/1.1" {
                let input = http_body(&request)["input"]
                    .as_str()
                    .expect("提交包含 input")
                    .to_owned();
                let marker = if input.contains("task-concurrent-b") {
                    "b"
                } else if input.contains("task-concurrent-a") {
                    "a"
                } else {
                    "large"
                };
                write_http_response(
                    &mut stream,
                    "application/json",
                    &format!(r#"{{"run_id":"run-{marker}","status":"started"}}"#),
                )
                .await;
            } else if first_line.contains("/events HTTP/1.1") {
                let marker = if first_line.contains("run-b") {
                    "b"
                } else if first_line.contains("run-a") {
                    "a"
                } else {
                    "large"
                };
                write_http_response(
                    &mut stream,
                    "text/event-stream",
                    &format!(
                        concat!(
                            "data: {{\"type\":\"run.started\"}}\n\n",
                            "data: {{\"type\":\"message.delta\",\"delta\":\"delta-{0}\"}}\n\n",
                            "data: {{\"type\":\"run.completed\",\"output\":\"done-{0}\"}}\n\n"
                        ),
                        marker
                    ),
                )
                .await;
            } else {
                panic!("收到未知 Hermes 请求：{first_line}");
            }
            requests.push(request);
        }
        requests
    });
    (format!("http://{address}"), task)
}

fn send_end_and_read_events(
    daemon: &TestDaemon,
    request_id: &str,
    task_id: &str,
) -> Vec<HandoffEvent> {
    let mut stream = UnixStream::connect(&daemon.paths.socket).expect("连接 daemon socket");
    write_json_frame(
        &mut stream,
        &envelope(daemon, end_request(request_id, task_id)),
    )
    .expect("发送 handoff_end");
    let mut events = Vec::new();
    loop {
        let event: HandoffEvent = read_json_frame(&mut stream).expect("读取 HandoffEvent");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_handoff_submits_large_body_without_truncation() {
    let (base_url, fake_task) = fake_hermes(3).await;
    let daemon = TestDaemon::start(&base_url);
    let task_id = "task-large-body";
    let markdown = format!("# 大正文\n\n{}", "large-body-line\n".repeat(30_000));
    let chunks = markdown
        .as_bytes()
        .chunks(MAX_HANDOFF_CHUNK_BYTES)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .expect("ASCII 分块应保持 UTF-8")
                .to_owned()
        })
        .collect::<Vec<_>>();

    assert!(chunks.len() >= 3, "大正文必须实际跨越多个 chunk");
    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(
                &daemon,
                "large-begin",
                task_id,
                markdown.as_bytes(),
                u32::try_from(chunks.len()).expect("chunk 数量可表示"),
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    for (index, chunk) in chunks.iter().enumerate() {
        assert_ack(
            send_value(
                &daemon,
                chunk_request(
                    &format!("large-chunk-{index}"),
                    task_id,
                    u32::try_from(index).expect("index 可表示"),
                    chunk,
                ),
            ),
            HandoffTransferPhase::Chunk,
            u32::try_from(index + 1).expect("next_index 可表示"),
        );
    }

    let events = send_end_and_read_events(&daemon, "large-end", task_id);
    assert!(events.iter().all(|event| event.task_id == task_id));
    assert_eq!(
        events.last().map(|event| event.event),
        Some(HandoffEventKind::Completed)
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].sequence == pair[0].sequence + 1)
    );

    let requests = fake_task.await.expect("等待 fake Hermes");
    let submission = requests
        .iter()
        .find(|request| String::from_utf8_lossy(request).starts_with("POST /v1/runs "))
        .expect("存在 Run 提交");
    let input = http_body(submission)["input"]
        .as_str()
        .expect("Run 提交包含 input")
        .to_owned();
    assert!(input.ends_with(&markdown), "Hermes 应收到无截断正文");
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn chunked_handoff_rejects_invalid_sequences_integrity_and_limits() {
    // fake Hermes 只用于证明所有失败都停在组装层，未产生任何远端请求。
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 Hermes 探针");
    let base_url = format!("http://{}", listener.local_addr().expect("读取探针地址"));
    let daemon = TestDaemon::start(&base_url);
    let body = "x".repeat(300);

    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(&daemon, "order-begin", "task-order", body.as_bytes(), 2),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    let (code, message) = failure(send_value(
        &daemon,
        valid_begin_request(
            &daemon,
            "order-repeated-begin",
            "task-order",
            body.as_bytes(),
            2,
        ),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("已存在"));
    let (code, message) = failure(send_value(
        &daemon,
        chunk_request("order-chunk", "task-order", 1, &body[..150]),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("顺序错误"));

    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(
                &daemon,
                "duplicate-begin",
                "task-duplicate",
                body.as_bytes(),
                2,
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    assert_ack(
        send_value(
            &daemon,
            chunk_request("duplicate-first", "task-duplicate", 0, &body[..150]),
        ),
        HandoffTransferPhase::Chunk,
        1,
    );
    let (code, message) = failure(send_value(
        &daemon,
        chunk_request("duplicate-repeat", "task-duplicate", 0, &body[150..]),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("顺序错误"));

    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(&daemon, "missing-begin", "task-missing", body.as_bytes(), 2),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    assert_ack(
        send_value(
            &daemon,
            chunk_request("missing-first", "task-missing", 0, &body[..150]),
        ),
        HandoffTransferPhase::Chunk,
        1,
    );
    let (code, message) = failure(send_value(
        &daemon,
        end_request("missing-end", "task-missing"),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("分块不完整"));

    assert_ack(
        send_value(
            &daemon,
            begin_request(
                &daemon,
                "sha-begin",
                "task-sha",
                body.as_bytes(),
                1,
                &"0".repeat(64),
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    assert_ack(
        send_value(&daemon, chunk_request("sha-chunk", "task-sha", 0, &body)),
        HandoffTransferPhase::Chunk,
        1,
    );
    let (code, message) = failure(send_value(&daemon, end_request("sha-end", "task-sha")));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("sha256"));

    let declared_larger = format!("{body}x");
    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(
                &daemon,
                "size-begin",
                "task-size",
                declared_larger.as_bytes(),
                1,
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    assert_ack(
        send_value(&daemon, chunk_request("size-chunk", "task-size", 0, &body)),
        HandoffTransferPhase::Chunk,
        1,
    );
    let (code, message) = failure(send_value(&daemon, end_request("size-end", "task-size")));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("大小不一致"));

    let oversized_chunk = "z".repeat(MAX_HANDOFF_CHUNK_BYTES + 1);
    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(
                &daemon,
                "chunk-limit-begin",
                "task-chunk-limit",
                oversized_chunk.as_bytes(),
                1,
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    let (code, _) = failure(send_value(
        &daemon,
        chunk_request("chunk-limit", "task-chunk-limit", 0, &oversized_chunk),
    ));
    assert_eq!(code, ErrorCode::MessageTooLarge);
    let (code, message) = failure(send_value(
        &daemon,
        chunk_request("chunk-after-limit", "task-chunk-limit", 0, &body),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("handoff_begin"));

    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(&daemon, "empty-begin", "task-empty", body.as_bytes(), 1),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    let (code, _) = failure(send_value(
        &daemon,
        chunk_request("empty-chunk", "task-empty", 0, ""),
    ));
    assert_eq!(code, ErrorCode::MessageTooLarge);
    let (code, message) = failure(send_value(
        &daemon,
        chunk_request("chunk-after-empty", "task-empty", 0, &body),
    ));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("handoff_begin"));

    let mut total_limit = valid_begin_request(
        &daemon,
        "total-limit",
        "task-total-limit",
        body.as_bytes(),
        1,
    );
    total_limit["command"]["total_bytes"] = json!(MAX_HANDOFF_CONTENT_BYTES + 1);
    let (code, _) = failure(send_value(&daemon, total_limit));
    assert_eq!(code, ErrorCode::MessageTooLarge);

    let mut chunks_limit = valid_begin_request(
        &daemon,
        "chunks-limit",
        "task-chunks-limit",
        body.as_bytes(),
        1,
    );
    chunks_limit["command"]["total_chunks"] = json!(MAX_HANDOFF_CHUNKS + 1);
    let (code, _) = failure(send_value(&daemon, chunks_limit));
    assert_eq!(code, ErrorCode::InvalidMessage);

    let mut invalid_task = valid_begin_request(&daemon, "bad-task", "task-ok", body.as_bytes(), 1);
    invalid_task["command"]["task_id"] = json!("");
    let (code, message) = failure(send_value(&daemon, invalid_task));
    assert_eq!(code, ErrorCode::InvalidMessage);
    assert!(message.contains("task_id"));

    for (request_id, field, value) in [
        ("bad-url", "url", json!("file:///tmp/private")),
        ("bad-title", "title", json!("  ")),
        ("bad-extractor", "extractor", json!("unknown")),
        ("bad-word-count", "word_count", json!(39)),
    ] {
        let mut request = valid_begin_request(
            &daemon,
            request_id,
            &format!("task-{request_id}"),
            body.as_bytes(),
            1,
        );
        request["command"]["source"][field] = value;
        let (code, message) = failure(send_value(&daemon, request));
        assert_eq!(code, ErrorCode::InvalidMessage);
        assert!(message.contains("元数据无效"));
    }

    // 无法可靠识别断开的 begin；满额应拒绝第 9 个任务，不能破坏任何已经 ACK 的任务。
    for index in 0..8 {
        assert_ack(
            send_value(
                &daemon,
                valid_begin_request(
                    &daemon,
                    &format!("capacity-begin-{index}"),
                    &format!("task-capacity-{index}"),
                    body.as_bytes(),
                    1,
                ),
            ),
            HandoffTransferPhase::Begin,
            0,
        );
    }
    let (code, message) = failure(send_value(
        &daemon,
        valid_begin_request(
            &daemon,
            "capacity-overflow",
            "task-capacity-overflow",
            body.as_bytes(),
            1,
        ),
    ));
    assert_eq!(code, ErrorCode::MessageTooLarge);
    assert!(message.contains("并发正文传输数量已达上限"));
    assert_ack(
        send_value(
            &daemon,
            chunk_request("capacity-still-present", "task-capacity-0", 0, &body),
        ),
        HandoffTransferPhase::Chunk,
        1,
    );
    let _ = failure(send_value(
        &daemon,
        chunk_request("capacity-cleanup-0", "task-capacity-0", 1, &body),
    ));
    for index in 1..8 {
        let _ = failure(send_value(
            &daemon,
            end_request(
                &format!("capacity-cleanup-{index}"),
                &format!("task-capacity-{index}"),
            ),
        ));
    }

    // begin 的 ACK 不代表提交 Run；连接立即关闭后，Hermes 不应收到请求。
    assert_ack(
        send_value(
            &daemon,
            valid_begin_request(
                &daemon,
                "disconnect-begin",
                "task-disconnected",
                body.as_bytes(),
                1,
            ),
        ),
        HandoffTransferPhase::Begin,
        0,
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err(),
        "仅 begin 或失败传输不得创建 Hermes Run"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn concurrent_chunked_handoffs_keep_chunks_and_events_isolated() {
    let (base_url, fake_task) = fake_hermes(6).await;
    let daemon = Arc::new(TestDaemon::start(&base_url));
    let body_a = format!("# A\n\n{}", "content-a\n".repeat(40));
    let body_b = format!("# B\n\n{}", "content-b\n".repeat(40));

    for (task_id, body) in [
        ("task-concurrent-a", &body_a),
        ("task-concurrent-b", &body_b),
    ] {
        assert_ack(
            send_value(
                &daemon,
                valid_begin_request(
                    &daemon,
                    &format!("{task_id}-begin"),
                    task_id,
                    body.as_bytes(),
                    2,
                ),
            ),
            HandoffTransferPhase::Begin,
            0,
        );
    }
    for index in 0..2 {
        for (task_id, body) in [
            ("task-concurrent-a", &body_a),
            ("task-concurrent-b", &body_b),
        ] {
            let midpoint = body.len() / 2;
            let data = if index == 0 {
                &body[..midpoint]
            } else {
                &body[midpoint..]
            };
            assert_ack(
                send_value(
                    &daemon,
                    chunk_request(&format!("{task_id}-{index}"), task_id, index, data),
                ),
                HandoffTransferPhase::Chunk,
                index + 1,
            );
        }
    }

    let event_sets = Arc::new(Mutex::new(Vec::new()));
    let mut readers = Vec::new();
    for task_id in ["task-concurrent-a", "task-concurrent-b"] {
        let daemon = Arc::clone(&daemon);
        let event_sets = Arc::clone(&event_sets);
        readers.push(tokio::task::spawn_blocking(move || {
            let events = send_end_and_read_events(&daemon, &format!("{task_id}-end"), task_id);
            event_sets.lock().expect("锁定事件集合").push(events);
        }));
    }
    for reader in readers {
        reader.await.expect("等待事件读取任务");
    }

    let event_sets = Arc::try_unwrap(event_sets)
        .expect("事件集合只剩一个引用")
        .into_inner()
        .expect("解锁事件集合");
    assert_eq!(event_sets.len(), 2);
    for events in &event_sets {
        let task_id = &events[0].task_id;
        assert!(events.iter().all(|event| &event.task_id == task_id));
        assert!(
            events
                .windows(2)
                .all(|pair| pair[1].sequence == pair[0].sequence + 1)
        );
        assert_eq!(
            events.last().map(|event| event.event),
            Some(HandoffEventKind::Completed)
        );
        let expected_marker = if task_id.ends_with('a') { "a" } else { "b" };
        assert!(
            events
                .iter()
                .filter_map(|event| event.text.as_deref())
                .all(|text| text.contains(expected_marker))
        );
    }

    let requests = fake_task.await.expect("等待 fake Hermes");
    let inputs = requests
        .iter()
        .filter(|request| String::from_utf8_lossy(request).starts_with("POST /v1/runs "))
        .map(|request| {
            http_body(request)["input"]
                .as_str()
                .expect("提交包含 input")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs
            .iter()
            .any(|input| input.ends_with(&body_a) && !input.contains(&body_b))
    );
    assert!(
        inputs
            .iter()
            .any(|input| input.ends_with(&body_b) && !input.contains(&body_a))
    );

    let daemon = Arc::try_unwrap(daemon).unwrap_or_else(|_| panic!("daemon 只剩一个引用"));
    daemon.stop().await;
}
