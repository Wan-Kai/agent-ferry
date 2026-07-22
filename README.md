# Agent Ferry

把浏览器里正在阅读的论文、帖子和文档，连同你写下的任务指令，直接交给本地 AI Agent 或自己的远程 Hermes。

Agent Ferry 不提供模型，也不会替你安装 Claude Code、Codex、OpenCode 或 Hermes。它只负责从浏览器安全地把当前页面交给你选择的 Agent。

## 开始使用

### 1. 安装 Chrome 扩展

[从 Chrome Web Store 安装 Agent Ferry](https://chromewebstore.google.com/detail/agent-ferry/ommpdijpcidnicpbalkpnggoljhapcel)

当前首发版本支持 macOS。

### 2. 安装并激活 Core

```bash
brew install Wan-Kai/tap/agent-ferry
aferry activate
```

`aferry activate` 会启动本地服务、连接 Chrome，并检测已经安装的本地 Agent：

```text
发现可连接的本地 Agent
  Claude Code  2.1.197
  OpenCode     1.17.18
  Codex        0.144.5

连接以上 Agent？ [Y/n]
```

直接按回车即可连接。首次连接会把执行 `aferry activate` 时所在的目录保存为默认运行位置；请先 `cd` 到你希望 Agent 启动的项目目录，或者显式指定：

```bash
aferry activate --yes --workspace '/path/to/your/project'
```

Agent Ferry 只连接已经存在且通过兼容性检查的 Agent。检测不到某个产品时，请先使用该产品的官方方式完成安装和登录。

### 3. 发送当前页面

打开任意网页，点击浏览器工具栏中的 Agent Ferry：

1. 选择 Agent。
2. 选择运行位置。
3. 检查或修改完整任务指令。
4. 点击开始。

页面正文只会在你点击开始后读取，并且只发送给你选择的 Agent。

## 连接远程 Hermes

如果 Hermes 使用官方 Docker 镜像部署在自己的服务器上，并且本机已经可以通过 SSH 公钥登录，只需要提供服务器地址：

```bash
aferry connect hermes root@example.com
```

Agent Ferry 会检查服务器、识别标准 Hermes 容器、建立安全通道并验证连接。连接名称默认根据服务器生成，也可以自定义：

```bash
aferry connect hermes root@example.com --name my-hermes
```

命令执行前会展示将要进行的远端变更并等待确认。完成后重新打开 Chrome 扩展，选择 Hermes 和对应的远程位置即可。

如果服务器包含多个 Hermes 容器、自定义 Docker 参数，或者已经公开了受保护的 Runs API，请参考[远程 Hermes 进阶配置](./docs/runbooks/installation.md#连接远程-hermes)。

## 当前支持

- Claude Code
- Codex CLI
- Codex App
- OpenCode
- 远程 Hermes
- 普通网页、X/Twitter、arXiv HTML 与 arXiv PDF

YouTube 专用提取仍在完善中。

## 检查状态

```bash
aferry doctor
```

查看本地服务日志：

```bash
aferry service logs --lines 100
```

## 升级

```bash
brew update
brew upgrade Wan-Kai/tap/agent-ferry
aferry activate
```

已有 Agent、运行位置和 Hermes 连接会保留；`activate` 只会提示新发现但尚未连接的 Agent。

## 卸载

保留配置、历史和 Hermes 凭据：

```bash
aferry uninstall
brew uninstall Wan-Kai/tap/agent-ferry
```

彻底删除全部 Agent Ferry 数据：

```bash
aferry uninstall --purge --yes
brew uninstall Wan-Kai/tap/agent-ferry
```

## 隐私与安全

- 网页正文不会经过 Agent Ferry 提供的云端服务。
- Hermes Token 保存在 macOS Keychain。
- Prompt、目标和运行位置在发送前对用户可见。
- Agent 仍以当前 macOS 用户权限运行；运行位置不是文件沙箱。

详细说明见[隐私政策](./PRIVACY.md)。

## 开发与架构

开发者入口：

- [当前架构](./docs/architecture/overview.md)
- [本地开发与验证](./docs/runbooks/development.md)
- [安装与故障处理](./docs/runbooks/installation.md)
- [真实环境验收](./docs/runbooks/real-environment-acceptance.md)

```bash
./scripts/verify
```
