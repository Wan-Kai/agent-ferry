#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use uuid::Uuid;

fn temporary_home() -> PathBuf {
    PathBuf::from(format!("/tmp/af-update-cli-{}", Uuid::new_v4().simple()))
}

fn installed_aferry(home: &Path) -> PathBuf {
    let version_root = home.join(".local/share/agent-ferry/versions/0.1.0");
    fs::create_dir_all(version_root.join("bin")).expect("创建版本 bin");
    fs::create_dir_all(version_root.join("share")).expect("创建版本 share");
    let binary = version_root.join("bin/aferry");
    fs::copy(env!("CARGO_BIN_EXE_aferry"), &binary).expect("复制 aferry");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("设置 aferry 权限");
    let installer = version_root.join("share/install.sh");
    fs::write(
        &installer,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/update-installer.log\"\nif [ -f \"$HOME/update-fail\" ]; then exit 42; fi\n",
    )
    .expect("写入测试安装器");
    fs::set_permissions(&installer, fs::Permissions::from_mode(0o755)).expect("设置安装器权限");
    binary
}

fn run(aferry: &Path, home: &Path, arguments: &[&str]) -> Output {
    Command::new(aferry)
        .env("HOME", home)
        .env_remove("AGENT_FERRY_HOME")
        .args(arguments)
        .output()
        .expect("运行 aferry update")
}

#[test]
fn update_uses_the_packaged_installer_and_selected_manifest() {
    let home = temporary_home();
    let aferry = installed_aferry(&home);
    fs::create_dir_all(home.join(".agent-ferry")).expect("创建数据目录");
    fs::write(
        home.join(".agent-ferry/install.json"),
        r#"{"version":"0.1.0","architecture":"arm64","manifest_url":"file:///default/release-manifest.json"}"#,
    )
    .expect("写入安装记录");

    let explicit = run(
        &aferry,
        &home,
        &[
            "update",
            "--manifest-url",
            "file:///selected/release-manifest.json",
        ],
    );
    assert!(
        explicit.status.success(),
        "显式更新失败: {}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    let default = run(&aferry, &home, &["update"]);
    assert!(default.status.success());
    let pinned = run(&aferry, &home, &["update", "--version", "0.2.0"]);
    assert!(pinned.status.success());

    let calls = fs::read_to_string(home.join("update-installer.log")).expect("读取安装器调用");
    assert!(calls.contains("--manifest-url file:///selected/release-manifest.json"));
    assert!(calls.contains("--manifest-url file:///default/release-manifest.json"));
    assert!(calls.contains(
        "--manifest-url https://github.com/Wan-Kai/agent-ferry/releases/download/v0.2.0/release-manifest.json"
    ));

    fs::write(home.join("update-fail"), "").expect("注入安装失败");
    let failed = run(&aferry, &home, &["update"]);
    assert_eq!(failed.status.code(), Some(42));

    fs::remove_dir_all(home).expect("清理测试目录");
}

#[test]
fn update_rejects_a_group_writable_packaged_installer() {
    let home = temporary_home();
    let aferry = installed_aferry(&home);
    let installer = home.join(".local/share/agent-ferry/versions/0.1.0/share/install.sh");
    fs::set_permissions(&installer, fs::Permissions::from_mode(0o775)).expect("放宽安装器权限");

    let output = run(
        &aferry,
        &home,
        &[
            "update",
            "--manifest-url",
            "file:///selected/release-manifest.json",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("不能被 group 或 other 写入"));
    assert!(!home.join("update-installer.log").exists());

    fs::remove_dir_all(home).expect("清理测试目录");
}
