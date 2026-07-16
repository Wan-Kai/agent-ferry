use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

#[must_use]
pub fn valid_ssh_host(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_whitespace)
}

#[derive(Debug, Clone)]
pub struct SshTunnelTransport {
    program: PathBuf,
    connect_timeout: Duration,
}

pub struct SshTunnel {
    local_port: u16,
    child: Child,
    stderr_task: tokio::task::JoinHandle<std::io::Result<u64>>,
}

impl SshTunnel {
    #[must_use]
    pub const fn local_port(&self) -> u16 {
        self.local_port
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        // Transport 生命周期只代表本地网络路径；上层 Agent 是否继续执行由协议适配器决定。
        let _ = self.child.start_kill();
        self.stderr_task.abort();
    }
}

impl SshTunnelTransport {
    #[must_use]
    pub fn system(connect_timeout: Duration) -> Self {
        Self {
            program: PathBuf::from("/usr/bin/ssh"),
            connect_timeout,
        }
    }

    #[must_use]
    pub fn with_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = program.into();
        self
    }

    /// 使用 OpenSSH 本地端口转发建立一条协议无关的 TCP Transport。
    ///
    /// # Errors
    ///
    /// host 无效、端口分配失败、SSH 启动/认证/Host Key 校验失败或超时时返回错误。
    pub async fn open(
        &self,
        ssh_host: &str,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<SshTunnel, SshTunnelError> {
        if !valid_ssh_host(ssh_host) {
            return Err(SshTunnelError::InvalidHost);
        }
        let reservation = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(SshTunnelError::Port)?;
        let local_port = reservation
            .local_addr()
            .map_err(SshTunnelError::Port)?
            .port();
        drop(reservation);

        let remote_forward_host = if remote_host.starts_with('[') && remote_host.ends_with(']') {
            remote_host.to_owned()
        } else if remote_host.contains(':') {
            format!("[{remote_host}]")
        } else {
            remote_host.to_owned()
        };
        let forward = format!("127.0.0.1:{local_port}:{remote_forward_host}:{remote_port}");
        let mut child = Command::new(&self.program)
            .args([
                "-N",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
            ])
            .args([
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
            ])
            // `--` 与输入校验共同防止持久配置被篡改后把 host 解释为 OpenSSH option。
            .args(["-L", &forward, "--", ssh_host])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| SshTunnelError::Spawn(self.program.clone(), error))?;

        let deadline = tokio::time::Instant::now() + self.connect_timeout;
        loop {
            if let Some(status) = child.try_wait().map_err(SshTunnelError::Wait)? {
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stderr.take() {
                    pipe.read_to_end(&mut stderr)
                        .await
                        .map_err(SshTunnelError::Wait)?;
                }
                return Err(SshTunnelError::Exited(
                    status.code(),
                    String::from_utf8_lossy(&stderr).trim().to_owned(),
                ));
            }
            if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(SshTunnelError::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let mut stderr = child.stderr.take().ok_or(SshTunnelError::MissingStderr)?;
        // 建立成功后的 stderr 仅用于防止 OpenSSH 管道反压；持续任务不能把日志无界积存在内存。
        let stderr_task =
            tokio::spawn(async move { tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await });
        Ok(SshTunnel {
            local_port,
            child,
            stderr_task,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshTunnelError {
    #[error("SSH host 必须是 ~/.ssh/config 中的单一 host、hostname 或 user@host，且不能以 - 开头")]
    InvalidHost,
    #[error("无法为 SSH Tunnel 分配本地端口: {0}")]
    Port(std::io::Error),
    #[error("无法启动系统 SSH {0}: {1}")]
    Spawn(PathBuf, std::io::Error),
    #[error("等待 SSH Tunnel 失败: {0}")]
    Wait(std::io::Error),
    #[error("SSH Tunnel 进程退出（code={0:?}）：{1}")]
    Exited(Option<i32>, String),
    #[error("SSH Tunnel 建立超时，请检查认证、Host Key 和远端地址")]
    Timeout,
    #[error("SSH Tunnel 未提供诊断输出管道")]
    MissingStderr,
}
