# Homebrew 安装 Agent Ferry

> 状态：Current
> 事实来源：`release/homebrew/agent-ferry.rb.in`、`scripts/build-homebrew-bottle`、
> `crates/agent-ferry-cli/src/{service,uninstall,update}.rs`
> 范围：macOS Core 的安装、升级和卸载；发布操作见 [发布 Runbook](./release.md)

## 支持范围

Agent Ferry 二进制最低部署目标仍为 macOS 11；官方 Homebrew 安装面以 Homebrew 当前支持的 macOS
版本为准。普通用户需要已有可运行的 Homebrew，但不需要 Rust、Node.js、Apple Developer 账号或
`sudo`。

```bash
brew install Wan-Kai/tap/agent-ferry
aferry activate
```

这里必须保留完整的 `Wan-Kai/tap/agent-ferry` 名称。Homebrew 6 会把这种直接安装视为只信任
目标 Formula，不会因此信任 Tap 中其他项目；用户不需要预先执行单独的 `brew tap` 或全局信任命令。

Formula 优先选择 GitHub Release 中与 CPU/系统匹配的原生 Bottle，Homebrew 在解压前验证 Formula
固定的 SHA-256。Apple Silicon 使用 `arm64_sonoma` Bottle，并可按 Homebrew 规则在更新 macOS 上
复用；Intel 使用 `sequoia` Bottle。Bottle 的 keg 只包含三个程序文件和 Homebrew 自身的安装元数据：

```text
bin/aferry
bin/agentferryd
bin/agentferry-host
```

没有匹配 Bottle 时，Formula 保留按架构选择的预编译 source fallback；它不编译 Rust，但 Homebrew
可能要求健康的 CLT。当前主要影响 Intel Sonoma，不能要求这类用户通过删除系统目录绕过检查。

三个 Mach-O 由 macOS CI 执行 ad-hoc 签名，满足 Apple Silicon 的可执行文件结构要求，但没有
Developer ID，也没有 Apple notarization。SHA-256 和 GitHub Artifact Attestation 证明发布内容
与对应 workflow/commit 的关系，不能冒充 Apple 对开发者身份的认可。

## 安装后激活

Homebrew 只安装程序文件，不在 `post_install` 沙箱中修改当前用户的 LaunchAgent 或 Chrome 配置。
安装完成后，在正常用户终端显式执行：

```bash
aferry activate
```

该命令从当前安装包中发现 `agentferryd` 与 `agentferry-host`，安装并启动当前用户的
`com.agentferry.daemon` LaunchAgent，并注册只允许正式 Chrome Item ID 连接的 Native Host。命令
幂等，中断后可以直接重试；它不会安装 Claude Code、Codex、OpenCode 或 Hermes，也不会读取
Hermes Keychain 凭据。安装后检查：

```bash
aferry service status
aferry doctor
```

早期开发版本的数据位于 `~/Library/Application Support/Agent Ferry`。Homebrew 不静默搬移开发
数据；需要保留时，先停止旧 daemon，再执行：

```bash
aferry service stop
aferry data migrate
aferry activate
```

双目录同时存在时迁移会拒绝覆盖。

## 升级

```bash
brew upgrade Wan-Kai/tap/agent-ferry
```

升级后再次执行 `aferry activate`，让 LaunchAgent 和 Native Host 指向新 keg，并重新加载新
daemon。旧 curl 安装专用的 `aferry update` 在 Homebrew 布局中只返回 `brew upgrade` 指引，不下载
或执行其他安装器。

```bash
aferry activate
```

## 卸载

先让 Ferry 删除它拥有的 LaunchAgent、Native Host manifest 和临时正文，再让 Homebrew 删除
程序：

```bash
aferry uninstall
brew uninstall Wan-Kai/tap/agent-ferry
```

第一条命令通过 canonical path 确认 plist 与 Native Host 确实指向当前 Homebrew keg；路径失效、
内容无效或属于其他安装时一律保留。它不会删除 Homebrew Cellar，也不会删除默认保留的：

- `~/.agent-ferry` 中的连接、Workspace、Prompt 和任务历史；
- `~/Library/Logs/Agent Ferry` 中的日志；
- macOS Keychain 中的 Hermes 凭据。

彻底清理：

```bash
aferry uninstall --purge --yes
brew uninstall Wan-Kai/tap/agent-ferry
```

`--purge` 必须与 `--yes` 同时使用。凭据删除失败时不会继续删除用户数据。

## 安全边界与故障处理

- Formula 只允许 `https://github.com/Wan-Kai/agent-ferry/releases/download/...` 的固定版本 URL；
- Formula 同时固定架构和 SHA-256，错误架构或任何字节变化都会在安装前失败；
- Formula 不使用 `post_install` 写用户目录；`aferry activate` 不使用 `sudo`、不下载其他 Agent；
- 安装流程不会执行 `xattr -d` 或修改系统 Gatekeeper 设置；
- 若企业 MDM 或 Gatekeeper 拒绝未公证程序，应停止安装并记录环境，不提供自动绕过命令；
- `aferry service logs --lines 100` 查看 daemon 日志；`aferry activate` 可重试激活。

## 本地验证

离线契约测试：

```bash
./scripts/test-homebrew-release
./scripts/test-homebrew-bottle-build
cargo test -p agent-ferry-cli --test uninstall_cli
cargo test -p agent-ferry-cli --test update_cli
```

真实 Homebrew 验收必须在隔离 HOME 或测试用户下安装生成的本地 Bottle，日志必须出现
`Pouring ...bottle.tar.gz`，并验证服务、Native Host、升级、卸载和日志；结果记录到
[真实环境验收](./real-environment-acceptance.md)。
