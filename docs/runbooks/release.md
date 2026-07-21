# Homebrew 与 GitHub Release 发布

> 状态：Current
> 事实来源：`.github/workflows/release.yml`、`scripts/build-macos-release`、
> `scripts/build-homebrew-bottle` 与 `release/homebrew/agent-ferry.rb.in`
> 范围：macOS Homebrew Core 和 Chrome ZIP；Chrome Web Store 审核见对应 Runbook

## 发布边界

本流程不需要 Apple Developer ID、证书或 notarization 凭据。手动 workflow 只生成 RC Artifact；
推送与源码版本一致的 `v<SemVer>` tag 才创建 GitHub Release 并更新 Homebrew Tap。

正式 Chrome Item ID 与 public key 必须已写入 `release/chrome-extension-identity.json`。Core Formula
中的 Native Host allowlist 和 Chrome ZIP 的 manifest key 只能由这同一文件生成。

## GitHub 配置

创建公开仓库 `Wan-Kai/homebrew-tap`，默认分支为 `main`。为它创建一对不带口令的独立
Ed25519 Deploy Key；这对密钥不能与个人 SSH 登录密钥或其他项目复用：

- 在 Tap 的 Settings → Deploy keys 中添加公钥，并仅为这枚 key 勾选 Allow write access；
- 在主仓库 Settings → Secrets and variables → Actions 中把私钥保存为
  `HOMEBREW_TAP_DEPLOY_KEY`；
- 私钥只保存在 GitHub Actions Secret 和创建时的受限临时文件，配置完成后删除临时文件。

Deploy Key 只能写 `Wan-Kai/homebrew-tap`，不能访问账号中的其他仓库。Release workflow 的
`GITHUB_TOKEN` 只负责当前仓库 Release 和 Artifact Attestation；流水线不使用个人 PAT，也不读取
Apple、Chrome 或 Hermes 凭据。

## 构建与完整性

`package` job 在 `macos-14` 上使用 Rust 1.85 构建最低 macOS 11 的 `arm64` 与 `x86_64` 三个
二进制。`scripts/package-homebrew-release` 对复制后的每个 Mach-O 执行 ad-hoc codesign 并严格
验证。随后两个原生 job 分别在 `macos-14` Apple Silicon 与 `macos-15-intel` 上用 Homebrew
生成 `arm64_sonoma` 和 `sequoia` Bottle。最终产物为：

```text
agent-ferry-v<version>-darwin-arm64.tar.gz
agent-ferry-v<version>-darwin-x86_64.tar.gz
agent-ferry--<version>.arm64_sonoma.bottle.tar.gz
agent-ferry--<version>.sequoia.bottle.tar.gz
agent-ferry-extension-v<version>-chrome.zip
agent-ferry.rb
checksums.txt
```

Formula 固定 Bottle root URL、双架构 tag/SHA-256、两个 fallback archive URL/SHA-256 和正式扩展
ID。发布证据记录完整 commit、Rust/Node 版本和所有产物 SHA-256；tag 构建还由 GitHub OIDC 生成
Artifact Attestation。

汇总后，macOS runner 创建临时 Tap，先用 `brew fetch --force-bottle` 禁止 source fallback，再用真实
`brew install` 安装同一 Formula，并要求日志出现 `Pouring ...bottle.tar.gz`。随后在隔离 HOME 中
验证服务状态、Native Host allowlist、日志读取、`aferry uninstall` 与 `brew uninstall`。
LaunchAgent 调用使用可观察的 fake `launchctl`，因此这条门禁能证明 Homebrew 与 Ferry 的进程/文件
契约，但不能替代真实用户会话中的 launchd smoke。脱敏输出保存为 `homebrew-e2e.log`，并由同一份
`verification.json` 绑定 SHA-256。

ad-hoc 签名不含发布者身份。文档、Release notes 和诊断不得将其描述为 Developer ID 签名或 Apple
公证。未来获得 Developer ID 后可以在打包前增加正式签名与公证，但不能降低当前哈希、证据和
Homebrew 所有权门禁。

## 发布操作

1. 保证源码版本、Chrome 版本和文档一致；
2. 在干净 commit 上运行 `./scripts/verify --require-clean`；
3. 手动运行 Release workflow，下载 RC Artifact；
4. 确认 RC 的两个 Bottle job 和真实 Bottle 安装、升级、卸载验收均通过；
5. 创建不可移动的新 tag：

```bash
git tag v0.1.1
git push origin v0.1.1
```

tag workflow 先创建不可变 GitHub Release，再 checkout `Wan-Kai/homebrew-tap`，复制生成的 Formula
并推送 `main`。Tap 更新失败时不得移动 tag 或替换 Release 内容；修复基础设施后，从已发布
Release 下载 `agent-ferry.rb` 并以单独 commit 恢复 Tap。

普通用户随后执行：

```bash
brew install Wan-Kai/tap/agent-ferry
```

## 验收证据

正式发布至少保存：

- 主仓库 tag 与完整 commit；
- GitHub Release 中所有资产的 SHA-256；
- GitHub Artifact Attestation；
- Tap `Formula/agent-ferry.rb` 对应 commit；
- RC Artifact 中的 `homebrew-e2e.log` 及其证据 SHA-256；
- `brew fetch --force-bottle`、`Pouring ...bottle.tar.gz` 与 `brew install` 的 SHA 校验结果；
- `aferry service status`、Native Host manifest 和 daemon 日志；
- 升级后进程切换到新 keg、卸载不删除用户数据的结果。

证据不得包含 Keychain secret、GitHub token、页面正文或用户 Prompt。
