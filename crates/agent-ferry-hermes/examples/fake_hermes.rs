use std::env;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CAPABILITIES: &str = r#"{
  "object": "hermes.api_server.capabilities",
  "platform": "hermes-agent",
  "model": "agent-ferry-e2e",
  "features": {
    "run_submission": true,
    "run_status": true,
    "run_events_sse": true,
    "run_stop": true,
    "run_approval_response": false
  }
}"#;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let address = env::var("FAKE_HERMES_ADDR").unwrap_or_else(|_| "127.0.0.1:18642".to_owned());
    let expected_token = env::var("FAKE_HERMES_TOKEN").unwrap_or_else(|_| "e2e-token".to_owned());
    let listener = TcpListener::bind(&address).await?;
    eprintln!("fake-hermes listening address={address}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let token = expected_token.clone();
        tokio::spawn(async move {
            if let Err(error) = serve(stream, &token).await {
                eprintln!("fake-hermes request_failed peer={peer} error={error}");
            }
        });
    }
}

async fn serve(mut stream: TcpStream, expected_token: &str) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&request);
    let first_line = request.lines().next().unwrap_or_default();
    let authorized = request
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&format!("authorization: Bearer {expected_token}")));
    let (status, body) = if first_line == "GET /v1/capabilities HTTP/1.1" && authorized {
        ("200 OK", CAPABILITIES)
    } else if !authorized {
        ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
    } else {
        ("404 Not Found", r#"{"error":"not_found"}"#)
    };

    // 日志只保留请求行与鉴权结果，确保端到端验证本身不会把测试 token 写入日志。
    eprintln!("fake-hermes request line={first_line:?} authorized={authorized}");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}
