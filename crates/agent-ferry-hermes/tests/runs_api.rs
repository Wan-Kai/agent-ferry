use std::time::Duration;

use agent_ferry_hermes::{HermesClient, HermesConnection};
use agent_ferry_protocol::HandoffEventKind;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug)]
struct CapturedRequest {
    head: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let read = stream
            .read(&mut buffer)
            .await
            .expect("读取 fake Hermes 请求");
        assert_ne!(read, 0, "HTTP 请求不应在 header 前结束");
        request.extend_from_slice(&buffer[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8(request[..header_end].to_vec()).expect("HTTP header 应为 UTF-8");
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length").then(|| {
                value
                    .trim()
                    .parse::<usize>()
                    .expect("Content-Length 应有效")
            })
        })
        .unwrap_or(0);
    while request.len() - header_end < content_length {
        let read = stream.read(&mut buffer).await.expect("读取 HTTP body");
        assert_ne!(read, 0, "HTTP body 不应提前结束");
        request.extend_from_slice(&buffer[..read]);
    }
    CapturedRequest {
        head,
        body: request[header_end..header_end + content_length].to_vec(),
    }
}

async fn write_json(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("写入 fake Hermes JSON 响应");
}

async fn write_sse_headers(stream: &mut TcpStream) {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("写入 fake Hermes SSE header");
}

fn assert_bearer(request: &CapturedRequest) {
    assert!(
        request
            .head
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer integration-token")),
        "请求必须携带期望的 Bearer Token: {}",
        request.head
    );
}

#[tokio::test]
async fn submits_input_and_delivers_chunked_sse_updates_in_order() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let server = tokio::spawn(async move {
        let (mut post_stream, _) = listener.accept().await.expect("接受 run submission");
        let post = read_request(&mut post_stream).await;
        write_json(
            &mut post_stream,
            "202 Accepted",
            r#"{"run_id":"run-integration","status":"started"}"#,
        )
        .await;

        let (mut event_stream, _) = listener.accept().await.expect("接受 SSE 订阅");
        let events = read_request(&mut event_stream).await;
        write_sse_headers(&mut event_stream).await;
        // 刻意跨 HTTP 写入切开 JSON 与 SSE 分隔符，覆盖真实网络中任意分块的情况。
        event_stream
            .write_all(b"data: {\"type\":\"message.")
            .await
            .expect("写入 SSE 第一分块");
        event_stream.flush().await.expect("刷新 SSE 第一分块");
        tokio::time::sleep(Duration::from_millis(20)).await;
        event_stream
            .write_all("delta\",\"delta\":\"第一段\"}\n".as_bytes())
            .await
            .expect("写入 SSE 第二分块");
        event_stream.flush().await.expect("刷新 SSE 第二分块");
        tokio::time::sleep(Duration::from_millis(20)).await;
        event_stream
            .write_all(
                "\ndata: {\"type\":\"run.completed\",\"output\":\"最终结果\"}\n\n".as_bytes(),
            )
            .await
            .expect("写入 SSE 终态分块");
        (post, events)
    });

    let connection = HermesConnection::direct("fake", &format!("http://{address}"), None)
        .expect("创建 fake Connection");
    let client = HermesClient::new(Duration::from_secs(2)).expect("创建 Hermes client");
    let mut receiver = client.run(
        connection,
        b"integration-token".to_vec(),
        "请分析这篇完整文档".to_owned(),
        true,
    );
    let mut updates = Vec::new();
    while let Some(update) = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("等待 Hermes Run 事件不应超时")
    {
        let terminal = update.kind == HandoffEventKind::Completed;
        updates.push(update);
        if terminal {
            break;
        }
    }

    assert_eq!(
        updates.iter().map(|update| update.kind).collect::<Vec<_>>(),
        [
            HandoffEventKind::Submitted,
            HandoffEventKind::OutputDelta,
            HandoffEventKind::Completed,
        ]
    );
    assert_eq!(updates[0].run_id.as_deref(), Some("run-integration"));
    assert_eq!(updates[1].text.as_deref(), Some("第一段"));
    assert_eq!(updates[2].text.as_deref(), Some("最终结果"));

    let (post, events) = server.await.expect("等待 fake Hermes 完成");
    assert!(post.head.starts_with("POST /v1/runs HTTP/1.1"));
    assert_bearer(&post);
    let body: Value = serde_json::from_slice(&post.body).expect("submission body 应为 JSON");
    assert_eq!(body, serde_json::json!({"input": "请分析这篇完整文档"}));
    assert!(
        events
            .head
            .starts_with("GET /v1/runs/run-integration/events HTTP/1.1")
    );
    assert_bearer(&events);
}

#[tokio::test]
async fn dropping_observer_never_stops_a_remote_run() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let server = tokio::spawn(async move {
        let (mut post_stream, _) = listener.accept().await.expect("接受 run submission");
        let post = read_request(&mut post_stream).await;
        write_json(
            &mut post_stream,
            "202 Accepted",
            r#"{"run_id":"run-detached","status":"started"}"#,
        )
        .await;

        let (mut event_stream, _) = listener.accept().await.expect("接受 SSE 订阅");
        let events = read_request(&mut event_stream).await;
        write_sse_headers(&mut event_stream).await;
        event_stream
            .write_all(b"data: {\"type\":\"run.started\"}\n\n")
            .await
            .expect("写入 run.started");
        tokio::time::sleep(Duration::from_millis(80)).await;

        // fake Hermes 的执行生命周期独立于观察连接；即使随后写流失败，远端仍已完成。
        let remote_completed = true;
        let _ = event_stream
            .write_all("data: {\"type\":\"run.completed\",\"output\":\"后台完成\"}\n\n".as_bytes())
            .await;

        let unexpected = tokio::time::timeout(Duration::from_millis(250), listener.accept()).await;
        let unexpected_request = if let Ok(Ok((mut stream, _))) = unexpected {
            Some(read_request(&mut stream).await)
        } else {
            None
        };
        (post, events, remote_completed, unexpected_request)
    });

    let connection = HermesConnection::direct("fake", &format!("http://{address}"), None)
        .expect("创建 fake Connection");
    let client = HermesClient::new(Duration::from_secs(2)).expect("创建 Hermes client");
    let mut receiver = client.run(
        connection,
        b"integration-token".to_vec(),
        "后台执行".to_owned(),
        true,
    );
    let submitted = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("等待 Submitted 不应超时")
        .expect("应收到 Submitted");
    assert_eq!(submitted.kind, HandoffEventKind::Submitted);
    drop(receiver);

    let (post, events, remote_completed, unexpected) =
        server.await.expect("等待 detached fake Run");
    assert_bearer(&post);
    assert_bearer(&events);
    assert!(remote_completed, "观察者离开后 fake 远端 Run 仍应完成");
    if let Some(request) = unexpected {
        assert!(
            !request.head.contains("/stop"),
            "关闭观察者不能触发 stop 请求: {}",
            request.head
        );
        panic!("关闭观察者后不应产生额外 HTTP 请求");
    }
}
