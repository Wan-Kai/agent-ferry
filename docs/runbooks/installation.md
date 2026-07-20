# macOS 安装与发行包

> 状态：Current
> 事实来源：`scripts/install.sh`、`scripts/package-macos-release` 与 `scripts/test-macos-installer`
> 范围：macOS Core 发行包和安装流程；Developer ID、公证与正式发布见发布 Runbook

## 用户安装契约

正式发布后的一行入口为：

```bash
curl -fsSL https://github.com/Wan-Kai/agent-ferry/releases/latest/download/install.sh | bash
```

需要固定版本时，同时下载该 Release 中已经绑定 Apple Team ID 的安装器，并把固定 manifest 作为参数传入：

```bash
curl -fsSLo /tmp/agent-ferry-install.sh \
  https://github.com/Wan-Kai/agent-ferry/releases/download/v<version>/install.sh
bash /tmp/agent-ferry-install.sh \
  --manifest-url https://github.com/Wan-Kai/agent-ferry/releases/download/v<version>/release-manifest.json
```

安装器只支持 macOS `arm64` 和 `x86_64`，不使用 `sudo`，普通用户不需要 Rust、Node.js 或 npm。

## 发布包结构

### 固定 Chrome 扩展身份

Native Messaging 的 `allowed_origins` 必须提前知道 Chrome 扩展 ID。正式发布不能使用某台机器上临时加载 unpacked Extension 产生的 ID，而要先把扩展 ZIP 上传到 Chrome Developer Dashboard 的草稿 Item（无需发布），从 Package 页面取得 Item ID 和 public key。

按 `release/chrome-extension-identity.example.json` 创建 `release/chrome-extension-identity.json`。`manifest_key` 是 public key 的 DER Base64，即去掉 PEM 标题、结尾和所有换行后的单行文本；它不是上传签名所用的 private key，可以进入发行配置。校验命令会按 Chrome 的算法从公钥推导 ID，并拒绝不匹配的组合：

```bash
"$(./scripts/find-node)" scripts/extension-identity.mjs \
  release/chrome-extension-identity.json
```

生成带固定身份的 Chrome Store ZIP：

```bash
./scripts/package-chrome-extension \
  --extension-identity release/chrome-extension-identity.json \
  --output-dir target/distribution
```

普通 `npm --prefix extension run build` 仍是无固定 ID 的开发构建；发行脚本调用 `zip:release`，缺少身份文件会直接失败。这样开发时不依赖商店账号，正式 ZIP、Core manifest 和 Native Host allowlist 又只能使用同一身份。

### macOS Core 包

`scripts/package-macos-release` 同时接收两个架构已经签名的二进制目录，生成：

```text
agent-ferry-v<version>-darwin-arm64.tar.gz
agent-ferry-v<version>-darwin-x86_64.tar.gz
release-manifest.json
checksums.txt
```

每个 archive 只包含：

```text
agent-ferry-v<version>-darwin-<architecture>/
├── bin/
│   ├── aferry
│   ├── agentferryd
│   └── agentferry-host
└── share/
    └── install.sh
```

示例：

```bash
./scripts/package-macos-release \
  --version 0.1.0 \
  --arm64-bin-dir target/release-arm64 \
  --x86_64-bin-dir target/release-x86_64 \
  --output-dir target/distribution \
  --base-url https://github.com/Wan-Kai/agent-ferry/releases/download/v0.1.0 \
  --extension-identity release/chrome-extension-identity.json \
  --team-id <APPLE_TEAM_ID>
```

Core 打包会把经过公钥校验的 `extension_id` 写入 `release-manifest.json`。安装器读取它后自动注册 Native Host，不要求普通用户查找或输入扩展 ID。`scripts/install.sh --extension-id` 仅保留给明确的开发验收，不用于正式发行。

## 安装步骤与安全边界

安装器依次执行：

1. 检查 macOS 和 CPU 架构；
2. 下载 `release-manifest.json` 和对应 archive；
3. 校验 manifest 中的 SHA256；
4. 使用 `codesign --verify --strict` 检查三个 Mach-O，并要求 Developer ID、Hardened Runtime 和内嵌 Apple Team ID 完全一致；
5. 验证 `aferry --version` 与 manifest 一致；
6. 拒绝覆盖非 Ferry 管理的 `~/.local/bin` 命令；
7. 停止旧服务并执行显式数据迁移；
8. 安装版本目录，原子切换 `current`，创建命令链接；
9. 安装 LaunchAgent，按需注册 Native Host，并检查服务状态；
10. 写入权限为 `0600` 的 `~/.agent-ferry/install.json`。

安装记录同时保存本次绑定的正式扩展 ID 和签名 Team ID，便于诊断安装来源。升级使用新 manifest 中的同一 ID 重写 Native Host manifest；若发布配置意外换成另一个 Item，扩展身份校验与发行审查应在产物到达用户前阻断。公开的 `install.sh` 由发行流程写入固定 Team ID；仓库中的未渲染模板会拒绝普通执行，避免本地开发脚本被误当成正式信任根。

安装根目录如果是符号链接会被拒绝；安装、更新和卸载通过 `~/.local/share/.agent-ferry.lock` 原子互斥。锁放在安装根目录之外，是为了让卸载进程删除整个程序目录时仍能保持互斥。任何一步失败都会恢复旧 `current`、旧版本目录、旧 plist、旧 Native Host manifest、旧数据位置和旧服务。首次安装失败会清除新版本、命令链接和新服务，保留诊断输出。

安装器会验证二进制签名结构；正式发布流水线还会验证 Developer ID Team、Hardened Runtime、Apple notarization、在线 Gatekeeper 和发布证据。不能把本地 ad-hoc 测试签名作为公开产物，具体流程见 [macOS 签名、公证与发布](./release.md)。

## 本地端到端验证

```bash
./scripts/test-macos-installer
```

测试会在隔离 HOME 中：

- 编译并 ad-hoc 签名三个真实 Mach-O；
- 生成双架构 archive、manifest 和 checksum；
- 安装并迁移带空格路径下的旧数据；
- 通过进程级 fake launchctl 验证服务和参数日志；
- 注册真实格式的 Native Host manifest；
- 验证发行 manifest、Native Host allowlist 和安装记录使用同一扩展 ID；
- 重复安装验证幂等；
- 注入 bootstrap 失败验证完整清理；
- 篡改 archive 验证 SHA256 阻断。

macOS 上的 `./scripts/verify` 会自动执行这条端到端测试；其他平台跳过，因为安装契约明确只支持 macOS。

## 升级

```bash
aferry update
aferry update --version 0.2.0
aferry update --manifest-url <release-manifest.json URL>
```

`aferry update` 不会从网络下载另一段安装脚本。它只执行当前已校验发行包中的
`share/install.sh`，并把 manifest URL 作为独立 argv 传入。脚本必须是普通可执行文件，不能是
符号链接，也不能允许 group 或 other 写入。

- 无参数时使用 `~/.agent-ferry/install.json` 保存的 manifest URL；
- `--version` 定位 GitHub 对应版本的固定 manifest；
- `--manifest-url` 用于镜像、开发或明确的版本源，不能与 `--version` 同时使用；
- 更新复用安装器的 SHA256、codesign、版本检查、原子 `current` 切换和完整失败恢复；
- 更新进程返回安装器的原始退出码，便于自动化判断。

端到端测试会给旧版本目录写入哨兵文件，再注入新服务 bootstrap 失败，验证 `current`、旧版本内容、旧 plist 和运行状态全部恢复。

## 卸载与彻底清理

日常卸载使用：

```bash
aferry uninstall
```

它会停止并移除 Agent Ferry 自己安装的 LaunchAgent，删除 Agent Ferry 自己注册的 Native Host manifest、`~/.local/bin` 命令链接和 `~/.local/share/agent-ferry` 程序目录，但保留：

- `~/.agent-ferry` 中的连接、Workspace、Prompt、任务历史和安装记录；
- `~/Library/Logs/Agent Ferry` 中的诊断日志；
- macOS Keychain 中由 Hermes Connection 引用的凭据。

普通卸载也会删除操作系统临时目录中的页面正文 Artifact。这些文件可能包含完整网页内容，且不属于
用户需要恢复的配置；如果临时命名空间或 Artifact 路径被替换成符号链接，卸载会拒绝继续，避免越界删除。

因此重新安装后可以继续使用原配置。若同一路径上的服务、Native Host manifest 或命令链接不属于当前 Agent Ferry 安装，卸载器会保留它们，不能借产品卸载越界删除用户文件。

只有用户明确希望放弃恢复能力时才使用：

```bash
aferry uninstall --purge --yes
```

`--purge` 会在停止服务后，额外删除当前连接引用的 Keychain 凭据、`~/.agent-ferry` 和日志目录。为避免脚本或误操作静默清空数据，它必须同时提供 `--yes`；单独使用任一参数都会失败。凭据清理失败时，卸载器不会继续删除数据，并会恢复此前正在运行的服务。

可以使用 `--json` 获取每个资源的删除、保留或外来文件状态，供自动化安装器展示结果：

```bash
aferry uninstall --json
aferry uninstall --purge --yes --json
```
