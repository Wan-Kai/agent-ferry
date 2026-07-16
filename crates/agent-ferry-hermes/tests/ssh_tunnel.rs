#![cfg(target_os = "macos")]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use agent_ferry_hermes::{DiagnosisState, HermesClient, HermesConnection};
use agent_ferry_protocol::HandoffEventKind;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

fn run(command: &mut Command, label: &str) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(
        output.status.success(),
        "{label}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn sshd_path() -> PathBuf {
    ["/opt/homebrew/sbin/sshd", "/usr/sbin/sshd"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .expect("需要本机 OpenSSH sshd")
}

async fn wait_for_port(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("sshd 未监听测试端口");
}

fn start_sshd(root: &Path, port: u16) -> Child {
    let user = std::env::var("USER").expect("USER");
    let host_key = root.join("host_key");
    let client_key = root.join("client_key");
    run(
        Command::new("/usr/bin/ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&host_key),
        "生成 host key",
    );
    run(
        Command::new("/usr/bin/ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&client_key),
        "生成 client key",
    );
    fs::copy(
        client_key.with_extension("pub"),
        root.join("authorized_keys"),
    )
    .expect("准备 authorized_keys");
    let server_config = root.join("sshd_config");
    fs::write(
        &server_config,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}/sshd.pid\nAuthorizedKeysFile {}/authorized_keys\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nUsePAM no\nStrictModes no\nAllowUsers {user}\nLogLevel ERROR\n",
            host_key.display(),
            root.display(),
            root.display(),
        ),
    )
    .expect("写入 sshd config");
    Command::new(sshd_path())
        .args(["-D", "-e", "-f"])
        .arg(server_config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 sshd")
}

fn write_ssh_wrapper(root: &Path, port: u16) -> PathBuf {
    let user = std::env::var("USER").expect("USER");
    let config = root.join("ssh_config");
    fs::write(
        &config,
        format!(
            "Host aferry-e2e\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  IdentityFile {}\n  IdentitiesOnly yes\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {}\n",
            root.join("client_key").display(),
            root.join("known_hosts").display(),
        ),
    )
    .expect("写入 client config");
    let wrapper = root.join("ssh-wrapper");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec /usr/bin/ssh -F '{}' \"$@\"\n",
            config.display()
        ),
    )
    .expect("写入 ssh wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).expect("设置 wrapper 权限");
    wrapper
}

async fn fake_hermes(listener: TcpListener) -> io::Result<()> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = stream.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let first_line = request
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let (content_type, body, terminal) = if first_line
            .starts_with(b"GET /v1/capabilities HTTP/1.1")
        {
            (
                "application/json",
                r#"{"object":"hermes.api_server.capabilities","platform":"hermes-agent","model":"ssh-e2e","features":{"run_submission":true,"run_status":true,"run_events_sse":true}}"#,
                false,
            )
        } else if first_line.starts_with(b"POST /v1/runs HTTP/1.1") {
            ("application/json", r#"{"run_id":"run-ssh-e2e"}"#, false)
        } else if first_line.starts_with(b"GET /v1/runs/run-ssh-e2e/events HTTP/1.1") {
            (
                "text/event-stream",
                "data: {\"type\":\"run.started\"}\n\ndata: {\"type\":\"message.delta\",\"delta\":\"经 SSH \"}\n\ndata: {\"type\":\"run.completed\",\"output\":\"完成\"}\n\n",
                true,
            )
        } else {
            continue;
        };
        let request_text = String::from_utf8_lossy(&request);
        assert!(
            request_text
                .to_ascii_lowercase()
                .contains("authorization: bearer e2e-token"),
            "Tunnel 后仍必须使用 Hermes Bearer Token"
        );
        if first_line.starts_with(b"POST /v1/runs HTTP/1.1") {
            assert!(request_text.contains("完整文档与可见 Prompt"));
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await?;
        if terminal {
            return Ok(());
        }
    }
}

#[tokio::test]
#[ignore = "需要本机 OpenSSH；发布验收时显式运行"]
async fn system_ssh_tunnel_carries_capabilities_runs_and_sse() {
    let root = std::env::temp_dir().join(format!("agent-ferry-ssh-e2e-{}", Uuid::new_v4()));
    fs::create_dir(&root).expect("创建测试目录");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("设置测试目录权限");
    let port_reservation = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("分配 sshd 端口");
    let sshd_port = port_reservation.local_addr().expect("sshd 地址").port();
    drop(port_reservation);
    let hermes_listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("启动假 Hermes");
    let hermes_port = hermes_listener.local_addr().expect("Hermes 地址").port();
    let hermes_task = tokio::spawn(fake_hermes(hermes_listener));
    let mut sshd = start_sshd(&root, sshd_port);
    wait_for_port(sshd_port).await;

    let connection = HermesConnection::ssh_tunnel(
        "ssh-e2e",
        &format!("http://127.0.0.1:{hermes_port}"),
        None,
        "aferry-e2e",
    )
    .expect("创建 SSH Connection");
    let client = HermesClient::new(Duration::from_secs(3))
        .expect("创建 Hermes client")
        .with_ssh_program(write_ssh_wrapper(&root, sshd_port));
    let diagnosis = client
        .diagnose(&connection, b"e2e-token")
        .await
        .expect("执行 capability discovery");

    let mut updates = client.run(
        connection,
        b"e2e-token".to_vec(),
        "完整文档与可见 Prompt".to_owned(),
        true,
    );
    let mut kinds = Vec::new();
    while let Some(update) = updates.recv().await {
        let terminal = matches!(
            update.kind,
            HandoffEventKind::Completed | HandoffEventKind::Failed
        );
        kinds.push(update.kind);
        if terminal {
            break;
        }
    }

    let _ = sshd.kill();
    let _ = sshd.wait();
    hermes_task
        .await
        .expect("假 Hermes task")
        .expect("假 Hermes IO");
    let _ = fs::remove_dir_all(root);
    assert_eq!(diagnosis.state, DiagnosisState::Ready);
    assert!(
        diagnosis
            .capabilities
            .iter()
            .any(|value| value == "run.events_sse")
    );
    assert_eq!(
        kinds,
        [
            HandoffEventKind::Submitted,
            HandoffEventKind::Running,
            HandoffEventKind::OutputDelta,
            HandoffEventKind::Completed,
        ]
    );
}
