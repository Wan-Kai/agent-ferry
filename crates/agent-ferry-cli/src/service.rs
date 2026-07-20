use std::env;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const SERVICE_LABEL: &str = "com.agentferry.daemon";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Running,
    Loaded,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceReport {
    pub state: ServiceState,
    pub pid: Option<u32>,
    pub label: &'static str,
    pub plist: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
}

pub struct ServiceManager {
    home: PathBuf,
    launchctl: PathBuf,
    plist: PathBuf,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl ServiceManager {
    pub fn discover() -> Result<Self, ServiceError> {
        if !cfg!(target_os = "macos") {
            return Err(ServiceError::UnsupportedPlatform);
        }
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(ServiceError::HomeDirectoryUnavailable)?;
        let launchctl = launchctl_path();
        Ok(Self {
            plist: home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{SERVICE_LABEL}.plist")),
            stdout_log: home
                .join("Library")
                .join("Logs")
                .join("Agent Ferry")
                .join("agentferryd.log"),
            stderr_log: home
                .join("Library")
                .join("Logs")
                .join("Agent Ferry")
                .join("agentferryd.error.log"),
            home,
            launchctl,
        })
    }

    pub fn install(&self, daemon_path: Option<&Path>) -> Result<ServiceReport, ServiceError> {
        let daemon = Self::resolve_daemon_path(daemon_path)?;
        self.prepare_directories()?;
        let previous_plist = match fs::read(&self.plist) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let uid = self.manager_uid()?;
        let was_loaded = self.status_for_uid(&uid)?.state != ServiceState::Stopped;
        self.write_plist(&daemon)?;
        let install_result = (|| {
            if was_loaded {
                self.run_checked([
                    OsStr::new("bootout"),
                    Self::service_target(&uid).as_os_str(),
                ])?;
            }
            self.run_checked([
                OsStr::new("bootstrap"),
                Self::domain_target(&uid).as_os_str(),
                self.plist.as_os_str(),
            ])?;
            Ok::<(), ServiceError>(())
        })();
        if let Err(install_error) = install_result {
            if let Err(rollback_error) =
                self.rollback_install(&uid, previous_plist.as_deref(), was_loaded)
            {
                return Err(ServiceError::InstallRollbackFailed {
                    install: Box::new(install_error),
                    rollback: Box::new(rollback_error),
                });
            }
            return Err(install_error);
        }
        self.status_for_uid(&uid)
    }

    pub fn start(&self) -> Result<ServiceReport, ServiceError> {
        if !self.plist.is_file() {
            return Err(ServiceError::NotInstalled(self.plist.clone()));
        }
        let uid = self.manager_uid()?;
        let status = self.status_for_uid(&uid)?;
        if status.state == ServiceState::Stopped {
            self.run_checked([
                OsStr::new("bootstrap"),
                Self::domain_target(&uid).as_os_str(),
                self.plist.as_os_str(),
            ])?;
        }
        self.status_for_uid(&uid)
    }

    pub fn stop(&self) -> Result<ServiceReport, ServiceError> {
        let uid = self.manager_uid()?;
        if self.status_for_uid(&uid)?.state != ServiceState::Stopped {
            self.run_checked([
                OsStr::new("bootout"),
                Self::service_target(&uid).as_os_str(),
            ])?;
        }
        self.status_for_uid(&uid)
    }

    pub fn restart(&self) -> Result<ServiceReport, ServiceError> {
        if !self.plist.is_file() {
            return Err(ServiceError::NotInstalled(self.plist.clone()));
        }
        let uid = self.manager_uid()?;
        if self.status_for_uid(&uid)?.state != ServiceState::Stopped {
            self.run_checked([
                OsStr::new("bootout"),
                Self::service_target(&uid).as_os_str(),
            ])?;
        }
        self.run_checked([
            OsStr::new("bootstrap"),
            Self::domain_target(&uid).as_os_str(),
            self.plist.as_os_str(),
        ])?;
        self.status_for_uid(&uid)
    }

    pub fn status(&self) -> Result<ServiceReport, ServiceError> {
        let uid = self.manager_uid()?;
        self.status_for_uid(&uid)
    }

    pub fn uninstall(&self) -> Result<ServiceReport, ServiceError> {
        let uid = self.manager_uid()?;
        if self.status_for_uid(&uid)?.state != ServiceState::Stopped {
            self.run_checked([
                OsStr::new("bootout"),
                Self::service_target(&uid).as_os_str(),
            ])?;
        }
        match fs::remove_file(&self.plist) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.status_for_uid(&uid)
    }

    pub fn logs(&self, lines: usize) -> Result<String, ServiceError> {
        let mut rendered = String::new();
        append_log_tail(&mut rendered, "stdout", &self.stdout_log, lines)?;
        append_log_tail(&mut rendered, "stderr", &self.stderr_log, lines)?;
        if rendered.is_empty() {
            rendered.push_str("尚无 agentferryd 日志\n");
        }
        Ok(rendered)
    }

    fn resolve_daemon_path(requested: Option<&Path>) -> Result<PathBuf, ServiceError> {
        let path = match requested {
            Some(path) => path.to_path_buf(),
            None => env::current_exe()?
                .parent()
                .ok_or(ServiceError::ExecutableParentUnavailable)?
                .join("agentferryd"),
        };
        if !path.is_absolute() {
            return Err(ServiceError::DaemonPathNotAbsolute(path));
        }
        let metadata = fs::metadata(&path).map_err(|source| ServiceError::DaemonUnavailable {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(ServiceError::DaemonNotExecutable(path));
        }
        Ok(path)
    }

    fn prepare_directories(&self) -> Result<(), ServiceError> {
        let data_root = self.home.join(".agent-ferry");
        let plist_parent = self
            .plist
            .parent()
            .ok_or(ServiceError::PathParentUnavailable(self.plist.clone()))?;
        let log_parent = self
            .stdout_log
            .parent()
            .ok_or(ServiceError::PathParentUnavailable(self.stdout_log.clone()))?;
        fs::create_dir_all(&data_root)?;
        fs::set_permissions(&data_root, fs::Permissions::from_mode(0o700))?;
        fs::create_dir_all(plist_parent)?;
        fs::create_dir_all(log_parent)?;
        Ok(())
    }

    fn write_plist(&self, daemon: &Path) -> Result<(), ServiceError> {
        let data_root = self.home.join(".agent-ferry");
        let path = format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            self.home.display()
        );
        let plist = render_plist(
            daemon,
            &data_root,
            &self.stdout_log,
            &self.stderr_log,
            &path,
        );
        self.write_plist_bytes(plist.as_bytes())
    }

    fn write_plist_bytes(&self, contents: &[u8]) -> Result<(), ServiceError> {
        let temporary = self
            .plist
            .with_extension(format!("plist.tmp-{}", Uuid::new_v4().simple()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.plist)?;
        fs::set_permissions(&self.plist, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    fn rollback_install(
        &self,
        uid: &str,
        previous_plist: Option<&[u8]>,
        was_loaded: bool,
    ) -> Result<(), ServiceError> {
        match previous_plist {
            Some(contents) => self.write_plist_bytes(contents)?,
            None => match fs::remove_file(&self.plist) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            },
        }

        if was_loaded && self.status_for_uid(uid)?.state == ServiceState::Stopped {
            if previous_plist.is_none() {
                return Err(ServiceError::PreviousPlistUnavailable);
            }
            self.run_checked([
                OsStr::new("bootstrap"),
                Self::domain_target(uid).as_os_str(),
                self.plist.as_os_str(),
            ])?;
        }
        Ok(())
    }

    fn manager_uid(&self) -> Result<String, ServiceError> {
        let output = self.run_checked([OsStr::new("manageruid")])?;
        let uid = String::from_utf8(output.stdout)
            .map_err(|_| ServiceError::InvalidManagerUid)?
            .trim()
            .to_owned();
        if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ServiceError::InvalidManagerUid);
        }
        Ok(uid)
    }

    fn status_for_uid(&self, uid: &str) -> Result<ServiceReport, ServiceError> {
        let output = Command::new(&self.launchctl)
            .arg("print")
            .arg(Self::service_target(uid))
            .output()
            .map_err(|source| ServiceError::LaunchctlUnavailable {
                path: self.launchctl.clone(),
                source,
            })?;
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            if output.status.code() == Some(113)
                || diagnostic.contains("Could not find service")
                || diagnostic.contains("service not found")
            {
                return Ok(self.report(ServiceState::Stopped, None));
            }
            return Err(ServiceError::LaunchctlFailed {
                arguments: format!("print {}", Self::service_target(uid).display()),
                status: output.status.code(),
                diagnostic: diagnostic.trim().to_owned(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pid = parse_launchctl_value(&stdout, "pid").and_then(|value| value.parse().ok());
        let state = if parse_launchctl_value(&stdout, "state") == Some("running") || pid.is_some() {
            ServiceState::Running
        } else {
            ServiceState::Loaded
        };
        Ok(self.report(state, pid))
    }

    fn report(&self, state: ServiceState, pid: Option<u32>) -> ServiceReport {
        ServiceReport {
            state,
            pid,
            label: SERVICE_LABEL,
            plist: self.plist.clone(),
            stdout_log: self.stdout_log.clone(),
            stderr_log: self.stderr_log.clone(),
        }
    }

    fn domain_target(uid: &str) -> PathBuf {
        PathBuf::from(format!("gui/{uid}"))
    }

    fn service_target(uid: &str) -> PathBuf {
        PathBuf::from(format!("gui/{uid}/{SERVICE_LABEL}"))
    }

    fn run_checked<I, S>(&self, arguments: I) -> Result<Output, ServiceError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_os_string())
            .collect::<Vec<_>>();
        let output = Command::new(&self.launchctl)
            .args(&arguments)
            .output()
            .map_err(|source| ServiceError::LaunchctlUnavailable {
                path: self.launchctl.clone(),
                source,
            })?;
        if output.status.success() {
            return Ok(output);
        }
        Err(ServiceError::LaunchctlFailed {
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            status: output.status.code(),
            diagnostic: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn launchctl_path() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(path) = env::var_os("AFERRY_LAUNCHCTL_PATH") {
        return PathBuf::from(path);
    }
    PathBuf::from("/bin/launchctl")
}

fn render_plist(
    daemon: &Path,
    working_directory: &Path,
    stdout_log: &Path,
    stderr_log: &Path,
    path: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{daemon}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{working_directory}</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>5</integer>
  <key>Umask</key>
  <integer>63</integer>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
  </dict>
  <key>StandardOutPath</key>
  <string>{stdout_log}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_log}</string>
</dict>
</plist>
"#,
        label = xml_escape(SERVICE_LABEL),
        daemon = xml_escape(&daemon.to_string_lossy()),
        working_directory = xml_escape(&working_directory.to_string_lossy()),
        path = xml_escape(path),
        stdout_log = xml_escape(&stdout_log.to_string_lossy()),
        stderr_log = xml_escape(&stderr_log.to_string_lossy()),
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_launchctl_value<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (candidate, value) = line.trim().split_once('=')?;
        (candidate.trim() == key).then(|| value.trim())
    })
}

fn append_log_tail(
    rendered: &mut String,
    label: &str,
    path: &Path,
    lines: usize,
) -> Result<(), ServiceError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let selected = contents.lines().rev().take(lines).collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(());
    }
    writeln!(rendered, "== {label}: {} ==", path.display()).expect("写入 String 不会失败");
    for line in selected.into_iter().rev() {
        rendered.push_str(line);
        rendered.push('\n');
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("aferry service 当前只支持 macOS")]
    UnsupportedPlatform,
    #[error("无法确定用户目录")]
    HomeDirectoryUnavailable,
    #[error("无法确定 aferry 可执行文件的父目录")]
    ExecutableParentUnavailable,
    #[error("路径缺少父目录: {0}")]
    PathParentUnavailable(PathBuf),
    #[error("agentferryd 路径必须是绝对路径: {0}")]
    DaemonPathNotAbsolute(PathBuf),
    #[error("无法读取 agentferryd: {path}: {source}")]
    DaemonUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("agentferryd 不可执行: {0}")]
    DaemonNotExecutable(PathBuf),
    #[error("尚未安装 Agent Ferry LaunchAgent: {0}")]
    NotInstalled(PathBuf),
    #[error("旧服务原本已加载，但没有可用于恢复的旧 plist")]
    PreviousPlistUnavailable,
    #[error("无法解析 launchd manager uid")]
    InvalidManagerUid,
    #[error("无法执行 launchctl {path}: {source}")]
    LaunchctlUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("launchctl {arguments} 失败（exit={status:?}）: {diagnostic}")]
    LaunchctlFailed {
        arguments: String,
        status: Option<i32>,
        diagnostic: String,
    },
    #[error("安装新服务失败且恢复旧服务也失败；安装错误: {install}；恢复错误: {rollback}")]
    InstallRollbackFailed {
        install: Box<ServiceError>,
        rollback: Box<ServiceError>,
    },
    #[error("文件操作失败: {0}")]
    Io(#[from] std::io::Error),
}
