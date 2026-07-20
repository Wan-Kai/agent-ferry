# macOS 签名、公证与发布

> 状态：Current
> 事实来源：`.github/workflows/release.yml`、`scripts/*macos-release` 与发行流程进程测试
> 范围：macOS Core 和 Chrome ZIP 的 RC/正式发布；不包含 Chrome Web Store 审核操作

## 发布边界

普通开发门禁不读取 Apple 或 Chrome 凭据。只有手动运行 Release workflow 或推送 `v<SemVer>` tag 时，GitHub `release` Environment 才能访问发布凭据。

- 手动 `workflow_dispatch`：生成带签名、公证和证据的 RC Artifact，不创建 GitHub Release；
- 推送与源码版本一致的 `v<SemVer>` tag：执行同一流程，通过后创建不可变 GitHub Release；
- 任一架构签名、公证、Gatekeeper、扩展身份或完整工程门禁失败，都不会发布。

正式扩展身份必须先按 [安装 Runbook](./installation.md#固定-chrome-扩展身份) 写入 `release/chrome-extension-identity.json`。

## GitHub Environment 配置

在仓库 Settings → Environments 新建 `release`，建议启用 Required reviewers。配置以下 Variables：

| Variable | 内容 |
|---|---|
| `APPLE_SIGNING_IDENTITY` | 完整的 `Developer ID Application: ... (TEAMID)` identity |
| `APPLE_TEAM_ID` | 10 位 Apple Developer Team ID |
| `APPLE_NOTARY_KEY_ID` | App Store Connect API Key ID |
| `APPLE_NOTARY_ISSUER_ID` | App Store Connect Issuer UUID |

配置以下 Secrets：

| Secret | 内容 |
|---|---|
| `APPLE_DEVELOPER_ID_P12_BASE64` | Developer ID Application 证书和私钥导出的 `.p12`，整体 Base64 |
| `APPLE_DEVELOPER_ID_P12_PASSWORD` | `.p12` 导出密码 |
| `APPLE_NOTARY_KEY_P8_BASE64` | App Store Connect API `.p8` 私钥，整体 Base64 |

P12 和 P8 只在临时 runner 文件与临时 Keychain 中出现，权限为 `0600`，workflow 的 `always()` 清理步骤会删除它们。公钥、Team ID、Key ID 和 Issuer ID 不是认证 Secret；真正敏感的是 P12/P8 私钥及 P12 密码。

workflow 会先运行 `scripts/check-release-environment`：只验证所有字段存在、公开 ID 的格式和两个文件 Secret 的 Base64 结构，不打印任何值。缺项会在开始编译前按变量名失败；P12 密码和私钥内容不得通过日志或聊天传递。

## 签名与公证流程

workflow 在同一 macOS runner 上交叉构建 `arm64` 与 `x86_64`，最低部署版本统一为 macOS 11。每个架构的 `aferry`、`agentferryd`、`agentferry-host` 都执行：

1. `Developer ID Application` 签名；
2. `--options runtime` 启用 Hardened Runtime；
3. `--timestamp` 获取安全时间戳；
4. 校验 Authority、TeamIdentifier 和 runtime flag；
5. 把三个已签名文件装入 ZIP，使用 App Store Connect API key 提交 `notarytool --wait`；
6. 要求 submission 与 log 都是 `Accepted`，且 log 没有 issue；
7. 对同一二进制执行在线 `spctl --assess --type execute`。

Apple 当前不能把 Notary ticket staple 到独立命令行二进制。workflow 公证包含这些二进制的 ZIP，使 ticket 发布到 Apple 在线服务；通过在线 Gatekeeper 后，再把字节未改变的文件放入最终 `tar.gz`。最终 archive 仍由安装器做 SHA256 和严格 codesign 校验。

## 产物与证据

正式 Release 包含：

```text
agent-ferry-v<version>-darwin-arm64.tar.gz
agent-ferry-v<version>-darwin-x86_64.tar.gz
agent-ferry-extension-v<version>-chrome.zip
release-manifest.json
checksums.txt
install.sh
install.sh.sha256
```

Actions RC Artifact 额外保留：

- 两个架构的 Notary submission JSON；
- 两个架构的 Notary log JSON；
- `macos.json`：Team ID、submission ID 与在线 Gatekeeper 结论；
- `verification.json`：完整 commit、工具链、工作区干净状态及全部产物 SHA256。

Notary 证据与公开产物被同一份 verification evidence 的 SHA256 绑定。证据不记录 P12、P8、密码、环境变量或完整 runner 日志。

## 发布操作

先运行手动 RC，下载 Actions Artifact 并完成真实环境验收。确认版本号已经同步到 Cargo workspace 和 `extension/package.json` 后，再创建并推送 tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

不要复用或移动已发布 tag。失败时修复源码、递增版本并重新生成完整 RC，不在已验收 archive 内替换单个文件。
