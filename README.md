# Agent Ferry

Agent Ferry 是一个本地优先的浏览器到 AI Agent 内容交接工具。

当前版本通过 Chrome 扩展提取网页实际内容，由本地 Rust daemon 将任务交给远程 Hermes，或在选定 Workspace 中启动 Claude Code、Codex 和 OpenCode。daemon 保存有界的本地任务历史，宿主 Agent 的认证、会话和文件仍由宿主自己管理。

当前实现与模块边界见 [CONTEXT.md](./CONTEXT.md) 和 [当前架构](./docs/architecture/overview.md)。早期 V0.1 方案与产品路线图保留为带状态的设计资料，不能替代当前代码事实。

数据处理与用户删除方式见 [隐私政策](./PRIVACY.md)。

## 当前目录

```text
crates/
  agent-ferry-core/      领域模型与核心流程
  agent-ferry-protocol/  Chrome Native Messaging 协议
  agent-ferry-cli/       aferry 命令行程序
  agent-ferry-host/      agentferry-host Native Messaging Host
  agent-ferry-daemon/    agentferryd 本地交接中枢
  agent-ferry-*/         Agent 与远程服务适配器
extension/               WXT + React + TypeScript Chrome 扩展
docs/                    当前架构、ADR、Runbook 与历史设计
scripts/                 上下文、工程门禁与证据工具
```

## 开发

完整开发约定见 [本地开发与验证](./docs/runbooks/development.md)。常用入口：

```bash
./scripts/context <目标路径...>
./scripts/verify
```

普通用户未来使用预编译产物，不需要安装 Rust 或 Node.js。

正式 Release 发布后，macOS 用户使用一条命令安装轻量 Core，不需要 `sudo`：

```bash
curl -fsSL https://github.com/Wan-Kai/agent-ferry/releases/latest/download/install.sh | bash
```

macOS 安装器、双架构发行包以及 Developer ID 签名、公证和 GitHub Release 流水线已经实现。正式运行仍需配置 Apple 发布凭据和 Chrome 固定扩展身份；仓库中的 `scripts/install.sh` 是等待发行流程写入 Apple Team ID 的模板，普通用户应使用 GitHub Release 中生成的安装器。

安装器行为和本地端到端验证见 [macOS 安装与发行包](./docs/runbooks/installation.md)，正式发布配置见 [macOS 签名、公证与发布](./docs/runbooks/release.md)。

Chrome 发行构建与 Core 发行包共用同一份扩展身份文件。正式 Item ID 与 public key 录入后，发行脚本会校验二者对应关系，并把同一个 ID 写入 Native Host allowlist：

```bash
./scripts/package-chrome-extension \
  --extension-identity release/chrome-extension-identity.json \
  --output-dir target/distribution
```

已安装用户通过当前发行包携带的受信任安装器升级：

```bash
aferry update
aferry update --version <version>
```

默认卸载只移除 Agent Ferry 程序、后台服务和 Native Host，保留用户数据、日志与凭据：

```bash
aferry uninstall
```

确认不再需要恢复配置、历史和远程 Hermes 连接时，才执行彻底清理：

```bash
aferry uninstall --purge --yes
```

## 用户数据目录

当前版本默认把配置、任务历史和运行状态保存在 `~/.agent-ferry`。`AGENT_FERRY_HOME` 只用于测试或显式的开发隔离。

早期开发构建使用过 `~/Library/Application Support/Agent Ferry`。停止旧的 `agentferryd` 后可以执行：

```bash
aferry data migrate
```

只有旧目录存在且新目录不存在时才会原子迁移；命令不会合并或覆盖两个已有目录。正式安装器将在启动新服务前执行这一迁移。

## macOS daemon

`agentferryd` 由当前用户的 LaunchAgent 管理，不需要管理员权限：

```bash
aferry service install
aferry service status
aferry service logs --lines 100
aferry service restart
aferry service uninstall
```

`service uninstall` 只移除后台服务；`aferry uninstall` 才是完整产品卸载入口。详细行为见 [macOS daemon 服务管理](./docs/runbooks/macos-service.md) 和 [macOS 安装与发行包](./docs/runbooks/installation.md)。
