use std::time::Duration;

use agent_ferry_hermes::{DiagnosisState, HermesClient, HermesConnection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn fake_server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动 fake Hermes");
    let address = listener.local_addr().expect("读取 fake Hermes 地址");
    let status = status.to_owned();
    let body = body.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("接受诊断请求");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream.read(&mut buffer).await.expect("读取 HTTP 请求");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("写入 HTTP 响应");
        String::from_utf8(request).expect("请求应为 UTF-8")
    });
    (format!("http://{address}"), task)
}

fn ready_capabilities() -> &'static str {
    r#"{
        "object":"hermes.api_server.capabilities",
        "platform":"hermes-agent",
        "model":"test-hermes",
        "features":{
            "run_submission":true,
            "run_status":true,
            "run_events_sse":true,
            "run_stop":false,
            "run_approval_response":false
        }
    }"#
}

#[tokio::test]
async fn distinguishes_ready_auth_network_and_incompatible_servers() {
    let client = HermesClient::new(Duration::from_secs(2)).expect("创建诊断 client");
    let secret = b"fake-hermes-token";

    let (ready_url, ready_task) = fake_server("200 OK", ready_capabilities()).await;
    let ready_connection =
        HermesConnection::direct("ready", &ready_url, None).expect("创建 Connection");
    let ready = client
        .diagnose(&ready_connection, secret)
        .await
        .expect("诊断成功服务");
    assert_eq!(ready.state, DiagnosisState::Ready);
    assert_eq!(
        ready.capabilities,
        ["run.submit", "run.status", "run.events_sse"]
    );
    let request = ready_task.await.expect("等待 fake Hermes");
    assert!(request.starts_with("GET /v1/capabilities HTTP/1.1"));
    assert!(request.contains("authorization: Bearer fake-hermes-token"));

    let (auth_url, auth_task) = fake_server("401 Unauthorized", r#"{"error":"invalid"}"#).await;
    let auth_connection =
        HermesConnection::direct("auth", &auth_url, None).expect("创建 Connection");
    let auth = client
        .diagnose(&auth_connection, secret)
        .await
        .expect("诊断认证失败服务");
    assert_eq!(auth.state, DiagnosisState::AuthenticationFailed);
    auth_task.await.expect("等待认证 fake Hermes");

    let incompatible_body = r#"{
        "object":"hermes.api_server.capabilities",
        "platform":"hermes-agent",
        "features":{"run_submission":true,"run_status":false}
    }"#;
    let (incompatible_url, incompatible_task) = fake_server("200 OK", incompatible_body).await;
    let incompatible_connection =
        HermesConnection::direct("old", &incompatible_url, None).expect("创建 Connection");
    let incompatible = client
        .diagnose(&incompatible_connection, secret)
        .await
        .expect("诊断不兼容服务");
    assert_eq!(incompatible.state, DiagnosisState::Incompatible);
    incompatible_task.await.expect("等待不兼容 fake Hermes");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("分配未监听端口");
    let unreachable_url = format!("http://{}", listener.local_addr().expect("读取端口"));
    drop(listener);
    let unreachable_connection =
        HermesConnection::direct("offline", &unreachable_url, None).expect("创建 Connection");
    let unreachable = client
        .diagnose(&unreachable_connection, secret)
        .await
        .expect("诊断离线服务");
    assert_eq!(unreachable.state, DiagnosisState::ConnectionFailed);

    for diagnosis in [ready, auth, incompatible, unreachable] {
        assert!(!diagnosis.detail.contains("fake-hermes-token"));
    }
}
