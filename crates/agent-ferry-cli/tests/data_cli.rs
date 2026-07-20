use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use uuid::Uuid;

fn temporary_home() -> PathBuf {
    PathBuf::from(format!("/tmp/af-data-cli-{}", Uuid::new_v4().simple()))
}

fn aferry(home: &PathBuf, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aferry"))
        .env("HOME", home)
        .env_remove("AGENT_FERRY_HOME")
        .args(arguments)
        .output()
        .expect("运行 aferry")
}

#[test]
fn data_migrate_moves_legacy_configuration_without_losing_files() {
    let home = temporary_home();
    let legacy = home
        .join("Library")
        .join("Application Support")
        .join("Agent Ferry");
    fs::create_dir_all(legacy.join("config")).expect("创建旧配置目录");
    fs::write(legacy.join("config/workspaces.json"), b"legacy").expect("写入旧配置");

    let output = aferry(&home, &["data", "migrate", "--json"]);

    assert!(
        output.status.success(),
        "迁移失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(stdout.contains("\"state\": \"migrated\""));
    assert_eq!(
        fs::read(home.join(".agent-ferry/config/workspaces.json")).expect("读取迁移结果"),
        b"legacy"
    );
    assert!(!legacy.exists());

    let repeated = aferry(&home, &["data", "migrate", "--json"]);
    assert!(repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stdout).contains("\"state\": \"not_needed\""));

    fs::remove_dir_all(home).expect("清理测试目录");
}

#[test]
fn data_migrate_reports_conflict_without_overwriting_either_root() {
    let home = temporary_home();
    let legacy = home
        .join("Library")
        .join("Application Support")
        .join("Agent Ferry");
    let current = home.join(".agent-ferry");
    fs::create_dir_all(&legacy).expect("创建旧目录");
    fs::create_dir_all(&current).expect("创建新目录");
    fs::write(legacy.join("legacy.txt"), b"legacy").expect("写入旧数据");
    fs::write(current.join("current.txt"), b"current").expect("写入新数据");

    let output = aferry(&home, &["data", "migrate", "--json"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("拒绝自动合并"));
    assert_eq!(
        fs::read(legacy.join("legacy.txt")).expect("旧数据仍存在"),
        b"legacy"
    );
    assert_eq!(
        fs::read(current.join("current.txt")).expect("新数据仍存在"),
        b"current"
    );

    fs::remove_dir_all(home).expect("清理测试目录");
}
