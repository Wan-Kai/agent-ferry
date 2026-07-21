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

普通用户使用 Homebrew 管理的架构专用预编译 Bottle，不需要安装 Rust、Node.js，也不需要
Apple Developer 账号。macOS 安装轻量 Core：

```bash
brew install Wan-Kai/tap/agent-ferry
aferry activate
```

需要已能正常运行的 Homebrew；安装预编译二进制，不要求 Rust、Node.js、Apple Developer 账号或
`sudo`。完整命令会在 Homebrew 6 中只信任目标 Formula，不会扩大为整个 Tap 的全局信任。

Formula 按 CPU 与 macOS 选择 GitHub macOS CI 生成的原生 Bottle，并在安装前校验固定 SHA-256；
没有匹配 Bottle 时才使用预编译 fallback 归档。Homebrew 只管理程序文件；用户随后执行幂等的
`aferry activate`，在正常终端环境中注册 Chrome Native Host 并启动当前用户的 LaunchAgent，整个
过程不使用 `sudo`。GitHub Release 同时发布构建来源证明，Chrome 固定扩展身份仍由同一发行门禁
校验。

安装、升级和卸载行为见 [Homebrew 安装](./docs/runbooks/installation.md)，正式发布配置见
[Homebrew 与 GitHub Release 发布](./docs/runbooks/release.md)。

Chrome 发行构建与 Core 发行包共用同一份扩展身份文件。正式 Item ID 与 public key 录入后，发行脚本会校验二者对应关系，并把同一个 ID 写入 Native Host allowlist：

```bash
./scripts/package-chrome-extension \
  --extension-identity release/chrome-extension-identity.json \
  --output-dir target/distribution
```

升级由 Homebrew 负责；升级后重新激活，让 daemon 和 Native Host 切换到新 keg：

```bash
brew upgrade Wan-Kai/tap/agent-ferry
aferry activate
```

默认卸载只移除 Agent Ferry 程序、后台服务和 Native Host，保留用户数据、日志与凭据：

```bash
aferry uninstall
brew uninstall Wan-Kai/tap/agent-ferry
```

确认不再需要恢复配置、历史和远程 Hermes 连接时，才执行彻底清理：

```bash
aferry uninstall --purge --yes
brew uninstall Wan-Kai/tap/agent-ferry
```

## 用户数据目录

当前版本默认把配置、任务历史和运行状态保存在 `~/.agent-ferry`。`AGENT_FERRY_HOME` 只用于测试或显式的开发隔离。

早期开发构建使用过 `~/Library/Application Support/Agent Ferry`。停止旧的 `agentferryd` 后可以执行：

```bash
aferry data migrate
```

只有旧目录存在且新目录不存在时才会原子迁移；命令不会合并或覆盖两个已有目录。Homebrew 不会
静默迁移开发数据，具体停止服务和迁移顺序见安装 Runbook。

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
