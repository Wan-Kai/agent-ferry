use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use uuid::Uuid;

fn temporary_root() -> PathBuf {
    PathBuf::from(format!("/tmp/af-workspace-cli-{}", Uuid::new_v4().simple()))
}

fn aferry(home: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("AGENT_FERRY_HOME", home)
        .args(arguments)
        .output()
        .expect("运行 aferry")
}

#[test]
fn add_list_diagnose_and_remove_never_delete_workspace_files() {
    let root = temporary_root();
    let home = root.join("ferry");
    let workspace = root.join("project");
    fs::create_dir_all(&workspace).expect("创建 Workspace");
    let sentinel = workspace.join("keep.txt");
    fs::write(&sentinel, "不能删除").expect("写入哨兵文件");
    let workspace_alias = root.join("project-alias");
    symlink(&workspace, &workspace_alias).expect("创建 Workspace 符号链接");

    let added = aferry(
        &home,
        &[
            "workspace",
            "add",
            "--name",
            "main-project",
            "--path",
            workspace_alias.to_str().expect("Workspace UTF-8"),
            "--json",
        ],
    );
    assert!(
        added.status.success(),
        "add 失败: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let stdout = String::from_utf8(added.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"state\": \"ready\""));
    assert!(stdout.contains("main-project"));

    let config = home.join("config/workspaces.json");
    assert_eq!(
        fs::metadata(&config)
            .expect("读取配置权限")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(home.join("config"))
            .expect("读取配置目录权限")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let config_text = fs::read_to_string(&config).expect("读取 Workspace 配置");
    assert!(config_text.contains(workspace.to_string_lossy().as_ref()));
    assert!(!config_text.contains(workspace_alias.to_string_lossy().as_ref()));
    let listed = aferry(&home, &["workspace", "list", "--json"]);
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("main-project"));

    let duplicate_name = aferry(
        &home,
        &[
            "workspace",
            "add",
            "--name",
            "main-project",
            "--path",
            workspace.to_str().expect("Workspace UTF-8"),
        ],
    );
    assert!(!duplicate_name.status.success());
    assert!(String::from_utf8_lossy(&duplicate_name.stderr).contains("名称已存在"));

    let duplicate_path = aferry(
        &home,
        &[
            "workspace",
            "add",
            "--name",
            "same-directory",
            "--path",
            workspace.to_str().expect("Workspace UTF-8"),
        ],
    );
    assert!(!duplicate_path.status.success());
    assert!(String::from_utf8_lossy(&duplicate_path.stderr).contains("路径已存在"));

    let removed = aferry(&home, &["workspace", "remove", "main-project"]);
    assert!(removed.status.success());
    assert!(sentinel.exists(), "移除配置不得删除 Workspace 文件");
    let listed = aferry(&home, &["workspace", "list", "--json"]);
    assert_eq!(String::from_utf8_lossy(&listed.stdout).trim(), "[]");
    fs::remove_dir_all(root).expect("清理测试目录");
}

#[test]
fn doctor_distinguishes_missing_not_directory_and_not_writable() {
    let root = temporary_root();
    let home = root.join("ferry");

    let missing = root.join("missing-later");
    fs::create_dir_all(&missing).expect("创建目录");
    assert!(
        aferry(
            &home,
            &[
                "workspace",
                "add",
                "--name",
                "missing",
                "--path",
                missing.to_str().expect("路径 UTF-8"),
            ],
        )
        .status
        .success()
    );
    fs::remove_dir(&missing).expect("删除目录");
    let diagnosis = aferry(&home, &["workspace", "doctor", "missing", "--json"]);
    assert!(!diagnosis.status.success());
    assert!(String::from_utf8_lossy(&diagnosis.stdout).contains("\"state\": \"missing\""));

    let changed = root.join("changed-to-file");
    fs::create_dir_all(&changed).expect("创建目录");
    assert!(
        aferry(
            &home,
            &[
                "workspace",
                "add",
                "--name",
                "changed",
                "--path",
                changed.to_str().expect("路径 UTF-8"),
            ],
        )
        .status
        .success()
    );
    fs::remove_dir(&changed).expect("删除目录");
    fs::write(&changed, "现在是文件").expect("替换为文件");
    let diagnosis = aferry(&home, &["workspace", "doctor", "changed", "--json"]);
    assert!(!diagnosis.status.success());
    assert!(String::from_utf8_lossy(&diagnosis.stdout).contains("\"state\": \"not_directory\""));

    let readonly = root.join("readonly");
    fs::create_dir_all(&readonly).expect("创建目录");
    assert!(
        aferry(
            &home,
            &[
                "workspace",
                "add",
                "--name",
                "readonly",
                "--path",
                readonly.to_str().expect("路径 UTF-8"),
            ],
        )
        .status
        .success()
    );
    fs::set_permissions(&readonly, fs::Permissions::from_mode(0o500)).expect("设置只读权限");
    let diagnosis = aferry(&home, &["workspace", "doctor", "readonly", "--json"]);
    assert!(!diagnosis.status.success());
    assert!(String::from_utf8_lossy(&diagnosis.stdout).contains("\"state\": \"not_writable\""));
    fs::set_permissions(&readonly, fs::Permissions::from_mode(0o700)).expect("恢复权限");
    fs::remove_dir_all(root).expect("清理测试目录");
}
