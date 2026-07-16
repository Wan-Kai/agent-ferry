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
    let expected_prompt = env::var("FAKE_HERMES_EXPECT_PROMPT").ok();
    let listener = TcpListener::bind(&address).await?;
    eprintln!("fake-hermes listening address={address}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let token = expected_token.clone();
        let prompt = expected_prompt.clone();
        tokio::spawn(async move {
            if let Err(error) = serve(stream, &token, prompt.as_deref()).await {
                eprintln!("fake-hermes request_failed peer={peer} error={error}");
            }
        });
    }
}

async fn serve(
    mut stream: TcpStream,
    expected_token: &str,
    expected_prompt: Option<&str>,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }

    let request = String::from_utf8_lossy(&request);
    let first_line = request.lines().next().unwrap_or_default();
    let authorized = request
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&format!("authorization: Bearer {expected_token}")));
    let prompt_matches = expected_prompt.map(|expected| {
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("input")?.as_str().map(ToOwned::to_owned))
            .is_some_and(|input| input.starts_with(&format!("{expected}\n\n---")))
    });
    let (status, content_type, body) = if first_line == "GET /v1/capabilities HTTP/1.1"
        && authorized
    {
        ("200 OK", "application/json", CAPABILITIES)
    } else if first_line == "POST /v1/runs HTTP/1.1" && authorized {
        (
            "202 Accepted",
            "application/json",
            r#"{"run_id":"run-browser-e2e","status":"started"}"#,
        )
    } else if first_line == "GET /v1/runs/run-browser-e2e/events HTTP/1.1" && authorized {
        (
            "200 OK",
            "text/event-stream",
            concat!(
                "data: {\"type\":\"run.started\"}\n\n",
                "data: {\"type\":\"message.delta\",\"delta\":\"已收到完整页面，正在分析。\"}\n\n",
                "data: {\"type\":\"run.completed\",\"output\":\"Chrome 端到端交接完成：正文与可见 Prompt 已进入 Hermes Run。\"}\n\n"
            ),
        )
    } else if !authorized {
        (
            "401 Unauthorized",
            "application/json",
            r#"{"error":"unauthorized"}"#,
        )
    } else {
        (
            "404 Not Found",
            "application/json",
            r#"{"error":"not_found"}"#,
        )
    };

    // 日志只保留请求行与鉴权结果，确保端到端验证本身不会把测试 token 写入日志。
    eprintln!(
        "fake-hermes request line={first_line:?} authorized={authorized} prompt_matches={prompt_matches:?}"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}
