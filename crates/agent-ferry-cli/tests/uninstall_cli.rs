#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::PathBuf;
use std::process::{Command, Output};

use agent_ferry_core::{AgentFerryPaths, NativeHostManifest};
use agent_ferry_hermes::{HermesConnection, HermesConnections};
use uuid::Uuid;

struct Fixture {
    home: PathBuf,
    aferry: PathBuf,
    launchctl: PathBuf,
    install_root: PathBuf,
    data_root: PathBuf,
    log_root: PathBuf,
    native_manifest: PathBuf,
    artifact_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = PathBuf::from(format!("/tmp/af-uninstall-cli-{}", Uuid::new_v4().simple()));
        let install_root = home.join(".local/share/agent-ferry");
        let version_root = install_root.join("versions/0.1.0");
        fs::create_dir_all(version_root.join("bin")).expect("创建版本 bin");
        let aferry = version_root.join("bin/aferry");
        fs::copy(env!("CARGO_BIN_EXE_aferry"), &aferry).expect("复制 aferry");
        fs::set_permissions(&aferry, fs::Permissions::from_mode(0o755)).expect("设置 aferry 权限");
        for binary in ["agentferryd", "agentferry-host"] {
            let path = version_root.join("bin").join(binary);
            fs::write(&path, "test binary").expect("写入测试二进制");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("设置执行权限");
        }
        symlink("versions/0.1.0", install_root.join("current")).expect("创建 current");
        let command_root = home.join(".local/bin");
        fs::create_dir_all(&command_root).expect("创建命令目录");
        for binary in ["aferry", "agentferryd", "agentferry-host"] {
            symlink(
                install_root.join("current/bin").join(binary),
                command_root.join(binary),
            )
            .expect("创建命令链接");
        }

        let launchctl = home.join("fake-launchctl");
        fs::write(
            &launchctl,
            "#!/bin/sh\ncase \"$1\" in\nmanageruid) echo 501;;\nprint) [ -f \"$HOME/launchctl-loaded\" ] && { echo 'state = running'; echo 'pid = 9876'; } || exit 113;;\nbootstrap) : > \"$HOME/launchctl-loaded\";;\nbootout) rm -f \"$HOME/launchctl-loaded\";;\n*) exit 64;;\nesac\n",
        )
        .expect("写入 fake launchctl");
        fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o700))
            .expect("设置 launchctl 权限");
        let fixture = Self {
            data_root: home.join(".agent-ferry"),
            log_root: home.join("Library/Logs/Agent Ferry"),
            native_manifest: home.join(
                "Library/Application Support/Google/Chrome/NativeMessagingHosts/com.agentferry.host.json",
            ),
            artifact_root: home.join("tmp/agent-ferry/artifacts"),
            home,
            aferry,
            launchctl,
            install_root,
        };
        fixture.prepare_service_and_data();
        fixture
    }

    fn prepare_service_and_data(&self) {
        let daemon = self.install_root.join("versions/0.1.0/bin/agentferryd");
        let installed = self.run(&[
            "service",
            "install",
            "--daemon-path",
            daemon.to_str().expect("daemon 路径 UTF-8"),
        ]);
        assert!(installed.status.success());
        fs::create_dir_all(self.data_root.join("config")).expect("创建配置目录");
        fs::create_dir_all(self.data_root.join("history")).expect("创建历史目录");
        fs::create_dir_all(&self.log_root).expect("创建日志目录");
        fs::write(self.log_root.join("agentferryd.log"), "keep log").expect("写入日志");
        fs::create_dir_all(&self.artifact_root).expect("创建临时正文目录");
        fs::write(
            self.artifact_root.join("captured-page.md"),
            "private page body",
        )
        .expect("写入临时正文");

        let connection = HermesConnection::direct("test", "https://example.com", None)
            .expect("创建 Hermes Connection");
        let connections = HermesConnections {
            connections: vec![connection.clone()],
        };
        let paths = AgentFerryPaths::from_root(self.data_root.clone());
        fs::write(
            &paths.hermes_connections,
            serde_json::to_vec(&connections).expect("编码 Connection"),
        )
        .expect("写入 Connection");
        let mut credentials = BTreeMap::new();
        credentials.insert(connection.credential_ref, b"test-token".to_vec());
        fs::create_dir_all(
            paths
                .development_credentials
                .parent()
                .expect("开发凭据包含父目录"),
        )
        .expect("创建开发凭据目录");
        fs::write(
            &paths.development_credentials,
            serde_json::to_vec(&credentials).expect("编码开发凭据"),
        )
        .expect("写入开发凭据");

        fs::create_dir_all(self.native_manifest.parent().expect("manifest 包含父目录"))
            .expect("创建 Native Host 目录");
        let manifest = NativeHostManifest::new(
            self.install_root.join("versions/0.1.0/bin/agentferry-host"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        fs::write(
            &self.native_manifest,
            serde_json::to_vec(&manifest).expect("编码 Native Host manifest"),
        )
        .expect("写入 Native Host manifest");
    }

    fn run(&self, arguments: &[&str]) -> Output {
        Command::new(&self.aferry)
            .env("HOME", &self.home)
            .env("TMPDIR", self.home.join("tmp"))
            .env("AGENT_FERRY_HOME", &self.data_root)
            .env("AFERRY_LAUNCHCTL_PATH", &self.launchctl)
            .args(arguments)
            .output()
            .expect("运行 aferry")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

#[test]
fn default_uninstall_removes_program_but_preserves_user_data_credentials_and_logs() {
    let fixture = Fixture::new();
    let credentials = fixture.data_root.join("dev/hermes-credentials.json");
    let credential_contents = fs::read(&credentials).expect("读取卸载前凭据");

    let output = fixture.run(&["uninstall", "--json"]);

    assert!(
        output.status.success(),
        "卸载失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.install_root.exists());
    assert!(!fixture.home.join(".local/bin/aferry").exists());
    assert!(!fixture.native_manifest.exists());
    assert!(
        !fixture
            .home
            .join("Library/LaunchAgents/com.agentferry.daemon.plist")
            .exists()
    );
    assert!(
        fixture
            .data_root
            .join("config/hermes-connections.json")
            .exists()
    );
    assert_eq!(
        fs::read(credentials).expect("凭据仍存在"),
        credential_contents
    );
    assert!(fixture.log_root.join("agentferryd.log").exists());
    assert!(!fixture.artifact_root.exists());
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"user_data\": \"preserved\""));
    assert!(stdout.contains("\"temporary_artifacts\": \"removed\""));
}

#[test]
fn purge_requires_confirmation_then_removes_data_credentials_and_logs() {
    let fixture = Fixture::new();

    let refused = fixture.run(&["uninstall", "--purge"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--purge --yes"));
    assert!(fixture.install_root.exists());
    assert!(fixture.data_root.exists());

    let purged = fixture.run(&["uninstall", "--purge", "--yes", "--json"]);
    assert!(
        purged.status.success(),
        "彻底卸载失败: {}",
        String::from_utf8_lossy(&purged.stderr)
    );
    assert!(!fixture.install_root.exists());
    assert!(!fixture.data_root.exists());
    assert!(!fixture.log_root.exists());
    assert!(!fixture.artifact_root.exists());
    assert!(String::from_utf8_lossy(&purged.stdout).contains("\"user_data\": \"purged\""));
}

#[test]
fn uninstall_preserves_foreign_command_and_native_host_manifest() {
    let fixture = Fixture::new();
    let foreign_command = fixture.home.join(".local/bin/agentferryd");
    fs::remove_file(&foreign_command).expect("移除 Ferry 命令链接");
    symlink("/tmp/foreign-agentferryd", &foreign_command).expect("创建外部命令链接");
    let foreign_manifest = NativeHostManifest::new(
        PathBuf::from("/tmp/foreign-agentferry-host"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    fs::write(
        &fixture.native_manifest,
        serde_json::to_vec(&foreign_manifest).expect("编码外部 manifest"),
    )
    .expect("替换 Native Host manifest");

    let output = fixture.run(&["uninstall", "--json"]);

    assert!(output.status.success());
    assert!(fs::symlink_metadata(&foreign_command).is_ok());
    assert!(fixture.native_manifest.exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"native_host\": \"preserved_foreign\"")
    );
}

#[test]
fn uninstall_refuses_to_race_an_active_installation() {
    let fixture = Fixture::new();
    let lock = fixture.home.join(".local/share/.agent-ferry.lock");
    fs::create_dir(&lock).expect("模拟安装锁");

    let output = fixture.run(&["uninstall"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("生命周期锁"));
    assert!(fixture.install_root.exists());
    assert!(fixture.data_root.exists());
}

#[test]
fn homebrew_uninstall_cleans_runtime_without_deleting_the_keg() {
    let home = PathBuf::from(format!(
        "/tmp/af-homebrew-uninstall-{}",
        Uuid::new_v4().simple()
    ));
    let keg_bin = home.join("homebrew/Cellar/agent-ferry/0.1.0/bin");
    fs::create_dir_all(&keg_bin).expect("创建模拟 Homebrew keg");
    let aferry = keg_bin.join("aferry");
    fs::copy(env!("CARGO_BIN_EXE_aferry"), &aferry).expect("复制 aferry");
    fs::set_permissions(&aferry, fs::Permissions::from_mode(0o755)).expect("设置 aferry 权限");
    for binary in ["agentferryd", "agentferry-host"] {
        let path = keg_bin.join(binary);
        fs::write(&path, "test binary").expect("写入模拟 Homebrew 二进制");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("设置执行权限");
    }
    let launchctl = home.join("fake-launchctl");
    fs::write(
        &launchctl,
        "#!/bin/sh\ncase \"$1\" in\nmanageruid) echo 501;;\nprint) [ -f \"$HOME/launchctl-loaded\" ] && { echo 'state = running'; echo 'pid = 9876'; } || exit 113;;\nbootstrap) : > \"$HOME/launchctl-loaded\";;\nbootout) rm -f \"$HOME/launchctl-loaded\";;\n*) exit 64;;\nesac\n",
    )
    .expect("写入 fake launchctl");
    fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o700))
        .expect("设置 launchctl 权限");

    let run = |arguments: &[&str]| {
        Command::new(&aferry)
            .env("HOME", &home)
            .env("TMPDIR", home.join("tmp"))
            .env("AGENT_FERRY_HOME", home.join(".agent-ferry"))
            .env("AFERRY_LAUNCHCTL_PATH", &launchctl)
            .args(arguments)
            .output()
            .expect("运行 Homebrew aferry")
    };
    let daemon = keg_bin.join("agentferryd");
    let installed = run(&[
        "service",
        "install",
        "--daemon-path",
        daemon.to_str().expect("daemon 路径 UTF-8"),
    ]);
    assert!(installed.status.success());

    let paths = AgentFerryPaths::from_root(home.join(".agent-ferry"));
    let native_manifest = home.join(
        "Library/Application Support/Google/Chrome/NativeMessagingHosts/com.agentferry.host.json",
    );
    fs::create_dir_all(native_manifest.parent().expect("manifest 父目录"))
        .expect("创建 Native Host 目录");
    fs::write(
        &native_manifest,
        serde_json::to_vec(&NativeHostManifest::new(
            keg_bin.join("agentferry-host"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ))
        .expect("编码 manifest"),
    )
    .expect("写入 manifest");
    fs::create_dir_all(&paths.root).expect("创建用户数据");

    let output = run(&["uninstall", "--json"]);

    assert!(
        output.status.success(),
        "Homebrew 清理失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(aferry.exists(), "aferry 必须继续由 Homebrew 管理");
    assert!(daemon.exists(), "daemon 必须继续由 Homebrew 管理");
    assert!(!native_manifest.exists());
    assert!(
        !home
            .join("Library/LaunchAgents/com.agentferry.daemon.plist")
            .exists()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"program\": \"managed_externally\"")
    );

    fs::remove_dir_all(home).expect("清理模拟 Homebrew home");
}
