#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use uuid::Uuid;

fn temporary_home() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/af-service-cli-{}-Agent Ferry & Test",
        Uuid::new_v4().simple()
    ))
}

fn make_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("写入测试可执行文件");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("设置可执行权限");
}

fn aferry(home: &Path, launchctl: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("HOME", home)
        .env("AGENT_FERRY_HOME", home.join(".agent-ferry"))
        .env("AFERRY_LAUNCHCTL_PATH", launchctl)
        .args(arguments)
        .output()
        .expect("运行 aferry")
}

fn assert_valid_plist(plist: &Path) {
    let plist_text = fs::read_to_string(plist).expect("读取 LaunchAgent plist");
    assert!(plist_text.contains("com.agentferry.daemon"));
    assert!(plist_text.contains("agentferryd-test"));
    assert!(plist_text.contains("Agent Ferry &amp; Test"));
    assert_eq!(
        fs::metadata(plist)
            .expect("读取 plist 权限")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let plist_lint = Command::new("/usr/bin/plutil")
        .args(["-lint", plist.to_str().expect("plist 路径 UTF-8")])
        .output()
        .expect("运行 plutil");
    assert!(
        plist_lint.status.success(),
        "plist 无效: {}",
        String::from_utf8_lossy(&plist_lint.stderr)
    );
}

#[test]
fn activate_discovers_packaged_binaries_and_is_idempotent() {
    let home = temporary_home();
    let bin = home.join("package/bin");
    fs::create_dir_all(&bin).expect("创建测试安装目录");
    let packaged_aferry = bin.join("aferry");
    fs::copy(env!("CARGO_BIN_EXE_aferry"), &packaged_aferry).expect("复制 aferry");
    let daemon = bin.join("agentferryd");
    let host = bin.join("agentferry-host");
    make_executable(&daemon, "#!/bin/sh\nexit 0\n");
    make_executable(&host, "#!/bin/sh\nexit 0\n");

    let launchctl = home.join("fake-launchctl");
    make_executable(
        &launchctl,
        r#"#!/bin/sh
case "$1" in
  manageruid) printf '%s\n' '501' ;;
  print)
    if [ -f "$HOME/launchctl-loaded" ]; then
      printf '%s\n' 'state = running' 'pid = 2468'
    else
      exit 113
    fi
    ;;
  bootstrap) : > "$HOME/launchctl-loaded" ;;
  bootout) rm -f "$HOME/launchctl-loaded" ;;
  *) exit 64 ;;
esac
"#,
    );

    for _ in 0..2 {
        let activated = Command::new(&packaged_aferry)
            .env("HOME", &home)
            .env("AGENT_FERRY_HOME", home.join(".agent-ferry"))
            .env("AFERRY_LAUNCHCTL_PATH", &launchctl)
            .args(["activate", "--json"])
            .output()
            .expect("运行 activate");
        assert!(
            activated.status.success(),
            "activate 失败: {}",
            String::from_utf8_lossy(&activated.stderr)
        );
        assert!(String::from_utf8_lossy(&activated.stdout).contains("\"state\":\"activated\""));
    }

    let plist = home.join("Library/LaunchAgents/com.agentferry.daemon.plist");
    let plist_program = Command::new("/usr/bin/plutil")
        .args(["-extract", "ProgramArguments.0", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .expect("读取 activate 生成的 plist");
    assert!(plist_program.status.success());
    assert_eq!(
        String::from_utf8(plist_program.stdout)
            .expect("plist program UTF-8")
            .trim(),
        daemon.to_str().expect("daemon 路径 UTF-8")
    );
    let manifest = home.join(
        "Library/Application Support/Google/Chrome/NativeMessagingHosts/com.agentferry.host.json",
    );
    let manifest_text = fs::read_to_string(&manifest).expect("读取 activate 生成的 manifest");
    assert!(manifest_text.contains("chrome-extension://ommpdijpcidnicpbalkpnggoljhapcel/"));
    assert!(manifest_text.contains(host.to_str().expect("host 路径 UTF-8")));

    fs::remove_dir_all(home).expect("清理测试目录");
}

#[test]
fn service_commands_manage_launch_agent_and_expose_logs() {
    let home = temporary_home();
    fs::create_dir_all(&home).expect("创建测试 home");
    let launchctl = home.join("fake-launchctl");
    make_executable(
        &launchctl,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/launchctl-arguments.log"
case "$1" in
  manageruid) printf '%s\n' '501' ;;
  print)
    if [ -f "$HOME/launchctl-loaded" ]; then
      printf '%s\n' 'state = running' 'pid = 4321'
    else
      exit 113
    fi
    ;;
  bootstrap) : > "$HOME/launchctl-loaded" ;;
  bootout) rm -f "$HOME/launchctl-loaded" ;;
  kickstart) : > "$HOME/launchctl-loaded" ;;
  *) exit 64 ;;
esac
"#,
    );
    let daemon = home.join("agentferryd-test");
    make_executable(&daemon, "#!/bin/sh\nexit 0\n");

    let installed = aferry(
        &home,
        &launchctl,
        &[
            "service",
            "install",
            "--daemon-path",
            daemon.to_str().expect("daemon 路径 UTF-8"),
            "--json",
        ],
    );
    assert!(
        installed.status.success(),
        "install 失败: {}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(String::from_utf8_lossy(&installed.stdout).contains("\"state\": \"running\""));

    let plist = home.join("Library/LaunchAgents/com.agentferry.daemon.plist");
    assert_valid_plist(&plist);

    let repeated_install = aferry(
        &home,
        &launchctl,
        &[
            "service",
            "install",
            "--daemon-path",
            daemon.to_str().expect("daemon 路径 UTF-8"),
        ],
    );
    assert!(repeated_install.status.success(), "重复安装必须幂等");

    let status = aferry(&home, &launchctl, &["service", "status", "--json"]);
    assert!(status.status.success());
    let status_json = String::from_utf8(status.stdout).expect("status stdout UTF-8");
    assert!(status_json.contains("\"state\": \"running\""));
    assert!(status_json.contains("\"pid\": 4321"));

    let stopped = aferry(&home, &launchctl, &["service", "stop"]);
    assert!(stopped.status.success());
    let stopped_status = aferry(&home, &launchctl, &["service", "status", "--json"]);
    assert!(!stopped_status.status.success());
    assert!(String::from_utf8_lossy(&stopped_status.stdout).contains("\"state\": \"stopped\""));

    assert!(
        aferry(&home, &launchctl, &["service", "start"])
            .status
            .success()
    );
    assert!(
        aferry(&home, &launchctl, &["service", "restart"])
            .status
            .success()
    );

    let stdout_log = home.join("Library/Logs/Agent Ferry/agentferryd.log");
    fs::write(&stdout_log, "旧日志\n最新日志\n").expect("写入 daemon 日志");
    let logs = aferry(&home, &launchctl, &["service", "logs", "--lines", "1"]);
    assert!(logs.status.success());
    let logs_text = String::from_utf8(logs.stdout).expect("logs stdout UTF-8");
    assert!(logs_text.contains("最新日志"));
    assert!(!logs_text.contains("旧日志"));

    let uninstalled = aferry(&home, &launchctl, &["service", "uninstall", "--json"]);
    assert!(uninstalled.status.success());
    assert!(!plist.exists());
    assert!(stdout_log.exists(), "卸载服务必须保留诊断日志");

    let launchctl_arguments =
        fs::read_to_string(home.join("launchctl-arguments.log")).expect("读取 launchctl 参数日志");
    assert!(launchctl_arguments.contains("bootstrap gui/501"));
    assert!(launchctl_arguments.contains("bootout gui/501/com.agentferry.daemon"));

    fs::remove_dir_all(home).expect("清理测试目录");
}

#[test]
fn failed_service_reinstall_restores_previous_plist_and_loaded_state() {
    let home = temporary_home();
    let launch_agents = home.join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents).expect("创建 LaunchAgents");
    let plist = launch_agents.join("com.agentferry.daemon.plist");
    fs::write(&plist, "previous plist").expect("写入旧 plist");
    fs::write(home.join("launchctl-loaded"), "").expect("模拟旧服务已加载");
    fs::write(home.join("fail-next-bootstrap"), "").expect("安排 bootstrap 失败");

    let launchctl = home.join("fake-launchctl");
    make_executable(
        &launchctl,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/launchctl-arguments.log"
case "$1" in
  manageruid) printf '%s\n' '501' ;;
  print)
    if [ -f "$HOME/launchctl-loaded" ]; then
      printf '%s\n' 'state = running' 'pid = 4321'
    else
      exit 113
    fi
    ;;
  bootstrap)
    if [ -f "$HOME/fail-next-bootstrap" ]; then
      rm -f "$HOME/fail-next-bootstrap"
      printf '%s\n' 'bootstrap failed' >&2
      exit 70
    fi
    : > "$HOME/launchctl-loaded"
    ;;
  bootout) rm -f "$HOME/launchctl-loaded" ;;
  *) exit 64 ;;
esac
"#,
    );
    let daemon = home.join("agentferryd-test");
    make_executable(&daemon, "#!/bin/sh\nexit 0\n");

    let output = aferry(
        &home,
        &launchctl,
        &[
            "service",
            "install",
            "--daemon-path",
            daemon.to_str().expect("daemon 路径 UTF-8"),
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("bootstrap failed"));
    assert_eq!(
        fs::read_to_string(&plist).expect("读取恢复的 plist"),
        "previous plist"
    );
    assert!(home.join("launchctl-loaded").exists(), "旧服务必须重新加载");
    let arguments =
        fs::read_to_string(home.join("launchctl-arguments.log")).expect("读取 launchctl 调用记录");
    assert_eq!(arguments.matches("bootstrap gui/501").count(), 2);

    fs::remove_dir_all(home).expect("清理测试目录");
}
