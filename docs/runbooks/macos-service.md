# macOS daemon 服务管理

> 状态：Current
> 事实来源：`agent-ferry-cli/src/service.rs` 与 `service_cli` 进程测试
> 范围：当前用户的 `agentferryd` LaunchAgent；完整产品卸载见安装 Runbook

## 安装与状态

预编译发行包中，`aferry` 会从自己的版本目录找到同目录的 `agentferryd`：

```bash
aferry service install
aferry service status
```

开发构建或非标准布局需要明确指定绝对路径：

```bash
aferry service install --daemon-path "$PWD/target/release/agentferryd"
```

安装会生成权限为 `0600` 的 `~/Library/LaunchAgents/com.agentferry.daemon.plist`，创建私有的 `~/.agent-ferry`，然后使用当前 GUI login domain 加载服务。重复安装会重新加载同一个 label，不会创建重复服务。

## 生命周期

```bash
aferry service start
aferry service stop
aferry service restart
aferry service status --json
aferry service uninstall
```

- `stop` 卸载当前 launchd job，但保留 plist，因此 `start` 可以再次加载；
- `uninstall` 停止 job 并删除 plist，但保留日志、用户配置、历史和 Keychain 凭据；
- 安装新 plist 后若 bootstrap 失败，CLI 会恢复旧 plist，并在旧服务原本已加载时重新加载旧服务；
- `status` 在服务停止时返回非零退出码，方便脚本和安装器进行健康判断。

## 日志

```bash
aferry service logs
aferry service logs --lines 20
```

日志保存在：

```text
~/Library/Logs/Agent Ferry/agentferryd.log
~/Library/Logs/Agent Ferry/agentferryd.error.log
```

`service uninstall` 不删除日志，也不删除程序。完整产品卸载使用 `aferry uninstall`；彻底删除用户数据、日志与相关凭据使用 `aferry uninstall --purge --yes`。具体安全边界见 [macOS 安装与发行包](./installation.md#卸载与彻底清理)。

## 故障检查

```bash
aferry service status --json
aferry service logs --lines 100
aferry doctor
```

LaunchAgent 使用显式 PATH，包含 `~/.local/bin`、Homebrew 常用目录和 macOS 系统目录。本地 Agent 仍应通过 `aferry agent ... bind` 保存绝对路径，避免依赖交互式 shell 初始化文件。
