# ADR 0032：使用 Homebrew 分发未公证的轻量 Core

## 状态

Accepted，2026-07-20。取代 [ADR 0031](./0031-cli-style-install-and-data-layout.md) 的程序安装与
发行渠道；保留其中用户数据与系统规定目录的边界。

## 背景

Agent Ferry 首发需要在没有 Apple Developer ID 的情况下提供可升级、可卸载且不要求用户安装
Rust 或 Node.js 的 macOS Core。源码 Formula 能避开预编译分发，但会给每位用户引入大型 Rust
构建依赖，与轻量目标冲突。直接 curl 安装未公证二进制又缺少包管理器提供的版本、哈希和所有权
边界。

## 决策

1. 官方安装入口为 `brew install Wan-Kai/tap/agent-ferry`；
2. GitHub macOS CI 同时构建 `arm64` 与 `x86_64` 的 `aferry`、`agentferryd` 和
   `agentferry-host`，复制后执行 ad-hoc 签名，不声明 Apple 开发者身份；
3. Homebrew Formula 按架构选择固定 GitHub Release URL 和 SHA-256，不在用户机器上安装 Rust；
4. Formula `post_install` 通过结构化 argv 调用 `aferry service install` 和
   `aferry native-host register`，不执行网络脚本、不使用 `sudo`；
5. 程序文件、升级和 keg 删除由 Homebrew 独占管理；Ferry 的卸载命令只删除自己拥有的服务、
   Native Host、临时正文和可选用户数据，不能删除 Homebrew Cellar；
6. 用户配置、历史和运行状态继续位于 `~/.agent-ferry`，LaunchAgent、日志和 Native Host
   manifest 继续使用 macOS 与 Chrome 规定的目录；
7. GitHub Release 保存双架构归档、Formula、checksums、Chrome ZIP、完整 commit 证据和
   Artifact Attestation；发布 workflow 随后原子更新 `Wan-Kai/homebrew-tap`；
8. 文档明确披露产物没有 Developer ID 与 Apple notarization。不得静默清除 quarantine，企业
   策略或 Gatekeeper 拒绝时应停止并解释，而不是绕过系统安全设置。

## 备选方案

### Homebrew 在用户机器上从源码构建

不采用为默认路径。供应链边界清晰，但会下载 Rust 并显著增加首次安装时间和磁盘占用。源码构建
仍可作为维护者诊断方式，而不是普通用户体验。

### 继续使用 curl 安装预编译包

不采用为官方入口。自研安装器可以做 SHA-256 和回滚，但还要自行承担版本解析、包所有权、升级
发现和卸载集成；在没有 Developer ID 时，Homebrew 是更成熟且更容易审查的交付边界。

### 等待 Developer ID 后再发布

不采用。申请状态不应阻塞现阶段的可验证测试；未来取得证书后可以在不改变 Homebrew 命令的前提
下把同一产物升级为 Developer ID 签名和公证版本。

## 后果

- 普通用户只需要已有的 Homebrew，不需要 Rust、Node.js、Apple 账号或管理员权限；
- Homebrew SHA-256 与 GitHub 构建证明提供来源和完整性证据，但不能替代 Apple 对开发者身份的背书；
- 安装、升级和删除程序分别使用 `brew install`、`brew upgrade`、`brew uninstall`；
- `aferry uninstall` 必须先清理由 Agent Ferry 管理的运行资源，随后提示 Homebrew 删除程序；
- `aferry update` 在 Homebrew 布局中必须引导用户执行 `brew upgrade`，不能查找旧 curl 安装器；
- Homebrew Tap 仓库和跨仓库写入 token 成为发布基础设施的一部分。
