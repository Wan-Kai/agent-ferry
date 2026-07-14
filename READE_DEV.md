# Agent Ferry 开发共识

> 状态：初始共识，2026-07-14
>
> 本文记录产品讨论中已经形成的结论、当前边界与尚未解决的问题。实现发生变化时，应同步更新本文或新增 ADR，避免代码与产品设计脱节。

## 1. 一句话定义

Agent Ferry 是一个本地优先的 Browser-to-Agent Handoff 工具：用户浏览网页时，一键把页面内容和自己的目标交接给指定工作区中的本地或远程 AI Agent。

这里的“目标”不限于研究和调研，也可以是开发、写作、分析、整理、验证或任何用户希望 Agent 接手完成的工作。

## 2. 命名

| 对象 | 名称 |
|---|---|
| 产品 | Agent Ferry |
| GitHub 仓库 | `agent-ferry` |
| 用户 CLI | `aferry` |
| Chrome Native Messaging Host | `agentferry-host` |
| 未来的常驻 Daemon | `agentferryd` |

CLI 使用较短的 `aferry`，避免与 crates.io 上已有的 `ferry`、`ferry-cli` 等包冲突。产品名和仓库名仍然使用 Agent Ferry。

## 3. 用户故事

### 3.1 初次安装

理想体验：

```text
安装 Agent Ferry
→ 注册 Chrome Native Messaging Host
→ 安装或引导安装 Chrome 扩展
→ 检测 Claude Code / Codex / OpenCode / Pi / Hermes
→ 创建一个或多个工作区配置
```

需要接受的浏览器约束：普通 CLI 不能在所有用户环境中静默安装 Chrome 扩展。正式分发优先考虑 Chrome Web Store；开发版可以使用 unpacked extension。`aferry setup` 可以自动注册 Native Messaging Host，并负责检查或引导扩展安装。

### 3.2 日常使用

```text
浏览网页
→ 打开扩展
→ 补充本次交接目标
→ 选择工作区
→ 选择 Agent 和启动模式
→ 页面内容保存为 Markdown
→ 在目标工作区启动 Agent
```

### 3.3 多工作区

工作区由用户创建和选择。一个用户可以配置多个工作区，例如：

- `~/kb`：知识整理；
- `~/projects/product-a`：产品 A 开发；
- `~/writing`：写作；
- `ssh://home-server/~/hermes-workspace`：远程 Hermes 工作区。

不同工作区可以配置不同的 inbox 目录、允许使用的 Agent、默认 Agent、默认启动模式和远程连接方式。

## 4. 首批支持的 Agent

首批目标范围：

- Claude Code；
- Codex；
- OpenCode；
- Pi；
- 本地 Hermes；
- 远程 Hermes。

Agent Ferry 不重新实现这些 Agent，也不接管它们的模型配置、订阅、认证、Skills、MCP 或项目指令。Terminal 模式下，这些内容完全由原生 CLI 管理；Managed 模式下，尽可能沿用 Agent 自己的配置和认证。

## 5. 产品边界

### 5.1 核心价值

Agent Ferry 聚焦三件事：

1. 从浏览器可靠地捕获有价值的内容；
2. 把内容放进正确的工作区，形成可审计、可复用的本地 artifact；
3. 用用户选择的方式启动或连接目标 Agent。

### 5.2 当前不做

早期版本不做以下事情：

- 自己实现 Agent Loop；
- 提供统一模型供应商网关；
- 多 Agent 编排、角色团队或 Agent-to-Agent 消息；
- Git worktree 自动分配、自动合并或完整开发流水线；
- Issue/Kanban/项目管理平台；
- 完整 Web Terminal 或 IDE；
- 云端账户和云端内容同步；
- 插件市场；
- 以 MCP 作为 Agent 控制协议。

这些能力分别更接近 Multica、Relaydeck 或完整 Agent 平台的范围。Agent Ferry 先保持为轻量、可靠的交接工具，而不是通用 Agent Fleet Manager。

## 6. 两条控制路径

社区实践表明，控制 Coding Agent 主要分成两条路线。Agent Ferry 应同时保留两条路径，不强行用一个接口掩盖能力差异。

### 6.1 Terminal 模式

Agent Ferry 在指定工作区中打开一个可见、可交互的原生终端，并启动用户选择的 Agent CLI。

特点：

- 保留 Agent 原生 TUI；
- 沿用用户已有订阅、登录和配置；
- 用户可以立即接管；
- 适配范围最广；
- Ferry 只负责启动，不承诺知道 Agent 的结构化状态。

这是 V0.1 的主路径。

### 6.2 Managed 模式

Agent Ferry 通过 ACP 或 Agent 原生 RPC 持有会话，负责消息、事件、权限审批、取消和恢复。

特点：

- 可以继续向同一个会话发送内容；
- 可以展示结构化进度和错误；
- 可以处理 Agent 的权限请求和澄清问题；
- 需要 Ferry 自己提供相应 UI；
- 需要常驻 `agentferryd` 持有子进程或连接。

这是 V0.2 之后的方向。统一层优先选择 ACP；只有 ACP 无法提供关键能力时，才增加 Agent 专用 RPC Controller。

## 7. 内容面与控制面

架构必须明确区分两个平面。

### 7.1 Content Plane

负责：

- 页面标题、URL、正文、选区和元数据；
- HTML 到 Markdown 的转换；
- artifact 命名、去重和落盘；
- 本地工作区写入；
- 通过 SSH 写入远程工作区；
- 后续可能支持图片、PDF 和附件。

Markdown 文件是最通用、最持久的内容接口。任何 Agent 都能读取文件，用户也可以在 Agent 运行前后检查和修改内容。

### 7.2 Control Plane

负责：

- 检测 Agent 是否安装；
- 启动新会话；
- 未来的继续发送、事件订阅、审批、取消、恢复和 attach；
- Terminal、ACP、原生 RPC 和远程连接的差异。

文件 inbox 不是控制协议。文件写入成功不等于 Agent 已经启动或已经读取文件。

## 8. V0.1 架构

```text
Chrome Extension
      │ Native Messaging
      ▼
agentferry-host（按需启动）
      │
      ▼
agent-ferry-core
      ├── 提取请求校验
      ├── 工作区解析
      ├── Markdown artifact 写入
      ├── Agent 检测与启动参数构造
      └── Local / SSH transport
              │
              ▼
      可见的原生 Terminal
              │
              ▼
 Claude / Codex / OpenCode / Pi / Hermes
```

### 8.1 为什么 V0.1 不需要 Daemon

V0.1 只负责写入内容和启动一个用户可见的终端。Chrome 可以按需启动 Native Messaging Host，任务完成后 Host 可以退出。

过早引入常驻 Daemon 只会增加安装、升级、日志、端口、安全和进程管理成本，却没有不可替代的产品价值。

### 8.2 何时引入 `agentferryd`

满足以下任一需求时，再增加 Daemon：

- 持有 ACP/RPC 长连接；
- 管理长期 Agent session；
- 后台执行任务；
- 接收流式事件；
- 从扩展继续向已有会话发送消息；
- 处理权限审批、取消和恢复；
- 管理 PTY 或远程持久连接。

## 9. 核心领域模型

初步领域对象：

```text
Handoff
├── id
├── captured_at
├── source
│   ├── title
│   ├── url
│   └── markdown
├── objective
├── workspace_id
├── agent_id
└── launch_mode
    ├── terminal
    ├── managed
    └── background
```

工作区：

```text
Workspace
├── id
├── name
├── location: local | ssh
├── root
├── inbox_dir
├── allowed_agents
├── default_agent
└── default_launch_mode
```

Agent Controller 应声明能力，而不是假设所有 Agent 功能相同：

```text
detect
launch_visible
start_session
send_message
stream_events
answer_permission
cancel
resume
attach
```

扩展只展示目标 Agent 和当前模式真正支持的操作。

## 10. Agent 接入策略

| Agent | V0.1 | Managed 首选 | 必要时的专用接口 |
|---|---|---|---|
| Claude Code | 原生终端 | ACP adapter | stream-json / Agent SDK |
| Codex | 原生终端 | ACP adapter | Codex app-server |
| OpenCode | 原生终端 | 原生 ACP | HTTP/OpenAPI/SSE |
| Pi | 原生终端 | ACP 或 Pi RPC | JSONL RPC |
| 本地 Hermes | 原生终端 | 原生 ACP | TUI Gateway |
| 远程 Hermes | SSH 交互终端 | SSH tunnel + ACP/WS | Hermes Gateway/API |

### 10.1 为什么不优先使用 PTY

PTY 可以托管真实 CLI，并允许 Ferry 注入键盘输入和展示终端画面，但无法稳定理解不同 TUI 的语义。Agent 更新、终端控制序列或权限界面变化都可能破坏控制逻辑。

PTY 可以作为未来可选的 `Managed Terminal` 后端，但不作为 V0.1 的基础，也不通过屏幕文本解析来假装获得可靠的结构化状态。

### 10.2 为什么 ACP 不是 MCP

- ACP 的方向是 Client/Editor 控制 Agent session；
- MCP 的方向是 Agent/Host 获取工具、资源和提示模板。

因此“不用 MCP 中转”的早期决策仍然成立。未来使用 ACP 管理 Agent，不与该决策冲突。

## 11. 本地与远程工作区

### 11.1 本地

- 直接写入工作区 inbox；
- 使用结构化 argv 启动 Agent；
- 避免拼接未经转义的 shell 字符串；
- 初始任务优先引用已写入的 artifact 路径，避免把大段网页正文塞进命令行。

### 11.2 远程 Hermes

V0.1：

- 通过 SSH/SCP 或 SFTP 写入 artifact；
- 使用带 TTY 的 SSH 会话启动远程 Hermes；
- 终端对用户可见并可接管。

未来 Managed 模式：

- 优先通过 SSH tunnel 连接 Hermes ACP、WebSocket Gateway 或 HTTP API；
- 不直接把未经保护的本地控制端口暴露到公网；
- 凭据引用保存在配置中，敏感值使用操作系统安全存储。

## 12. Chrome 扩展

技术选型：

- WXT；
- React；
- TypeScript；
- Chrome Manifest V3；
- Native Messaging 与 Rust Host 通信。

扩展职责：

- 提取当前页面标题、URL、正文和用户选区；
- 转换或预处理为 Markdown；
- 收集 objective、workspace、agent 和 launch mode；
- 展示写入和启动结果；
- 不直接监听未经认证的 localhost HTTP 端口；
- 不持有 Agent API Key。

Node.js 只用于扩展开发和构建。正式用户安装预构建扩展，不需要 Node.js 运行环境。

## 13. Rust 工程

选择 Rust 的原因：

- 项目本身是学习 Rust 的实践载体；
- 适合构建无运行时依赖的跨平台 CLI、Native Host 和未来 Daemon；
- 对进程、IPC、文件系统和并发控制有良好支持；
- 可以从同一代码库发布 macOS、Linux 和 Windows 二进制。

初始 workspace：

```text
crates/
  agent-ferry-core/      不依赖 UI 的领域模型和用例
  agent-ferry-protocol/  Native Messaging 请求与响应
  agent-ferry-cli/       aferry setup/config/doctor 等命令
  agent-ferry-host/      Chrome Native Messaging Host
```

未来只有在确实需要常驻会话时才增加：

```text
crates/agent-ferry-daemon/
```

早期不为了“异步可能有用”而默认引入 Tokio。等到 ACP 长连接、并发 session 或 Daemon 出现后，再根据具体需求选择异步运行时。

## 14. 安全约束

网页内容是不可信输入，可能包含 prompt injection、恶意文件名、超长内容和终端控制字符。

最低安全要求：

- 页面正文只作为用户 artifact，不作为 Ferry 自己的控制指令；
- 文件名必须清洗并限制长度；
- 所有目标路径必须验证位于选定工作区之内；
- 不允许网页内容影响可执行文件、argv、环境变量或工作区选择；
- 启动命令使用结构化参数，不拼接 shell 命令字符串；
- Native Messaging manifest 只允许正式扩展 ID；
- 日志不得记录 token、密码或完整隐私内容；
- 远程连接默认走 SSH，不公开裸控制端口；
- Managed 模式必须正确呈现权限审批，不能默认静默放行危险操作。

## 15. 发布与安装

用户下载预编译产物，不要求安装 Rust 或 Node.js。

计划的发布渠道：

1. GitHub Releases：所有平台的基础分发渠道；
2. Homebrew：macOS/Linux 的首选安装方式；
3. WinGet：Windows 稳定后加入；
4. `cargo install`：主要面向 Rust 开发者和源码安装。

计划发布目标：

- macOS Apple Silicon；
- macOS Intel（根据实际用户需求决定保留时间）；
- Linux x86_64；
- Linux ARM64；
- Windows x86_64 MSVC。

Windows MSVC 正式产物使用 GitHub Actions Windows Runner 原生构建和测试。macOS 到 MSVC 的交叉编译可以用于实验，但不作为唯一正式发布链路。

## 16. 路线图

### V0.1：可靠交接

- Chrome 扩展捕获页面；
- Markdown artifact；
- 多工作区；
- Agent 自动检测；
- Claude Code、Codex、OpenCode、Pi、本地 Hermes；
- 远程 Hermes SSH 模式；
- 可见原生终端启动；
- `aferry setup`、`aferry doctor`、`aferry config`；
- Native Messaging Host 注册；
- GitHub Releases 和 Homebrew 安装。

### V0.2：托管会话

- 可选 `agentferryd`；
- ACP Client；
- Managed Session；
- 流式状态；
- 继续发送；
- 权限审批；
- 取消与恢复；
- 必要的 Agent 专用 RPC Controller。

### 更远期候选

- 图片、PDF 和附件；
- 浏览器侧内容模板；
- workspace routing rules；
- 现有 session attach；
- 后台运行；
- 移动端或其他浏览器入口。

这些候选不自动进入承诺范围，需要根据真实使用反馈重新排序。

## 17. 社区方案带来的结论

- Zed 同时保留 Terminal Threads 和 ACP External Agents，说明原生终端体验与托管会话是互补关系；
- Relaydeck 证明 PTY 可以统一托管真实 CLI，但也代表更重的 Fleet Manager 产品形态；
- Multica 证明 Daemon 适合后台任务、隔离 workspace 和长时间执行，但这些不是 Ferry V0.1 的必要条件；
- OpenCode、Codex、Pi 和 Hermes 分别提供不同的结构化接口，说明专用集成能力很强，但无法依赖一个统一的私有协议；
- ACP 是目前最适合作为通用 Managed Control Plane 的社区协议；
- 文件仍然是最通用的 Content Plane。

参考资料：

- [Zed Terminal Threads](https://zed.dev/docs/ai/terminal-threads)
- [Zed External Agents](https://zed.dev/docs/ai/external-agents)
- [Agent Client Protocol](https://github.com/agentclientprotocol/agent-client-protocol)
- [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp)
- [Codex app-server](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)
- [OpenCode Server](https://opencode.ai/docs/server/)
- [OpenCode ACP](https://opencode.ai/docs/acp/)
- [Pi RPC](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/rpc.md)
- [Hermes Programmatic Integration](https://github.com/nousresearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)
- [Multica CLI and Daemon](https://github.com/multica-ai/multica/blob/main/CLI_AND_DAEMON.md)
- [Relaydeck](https://relaydeck.ai/)

## 18. 尚未决定

以下问题仍然开放，不能在实现中悄悄假设：

- 开源许可证；
- Chrome Web Store 的发布主体和扩展 ID；
- V0.1 默认 inbox 文件命名规则；
- HTML 到 Markdown 的具体实现库；
- macOS 默认终端，以及 iTerm2、Warp 等终端的支持范围；
- Windows Terminal 和 Linux terminal launcher 的优先级；
- SSH 配置是复用 `~/.ssh/config`，还是额外维护 Ferry profile；
- Handoff 历史使用纯文件、JSON 还是 SQLite；
- V0.2 的 ACP session 是否由扩展直接呈现，还是增加独立桌面/Web UI。

## 19. 开发原则

- 先完成纵向闭环，再扩充 Agent 数量和高级能力；
- 核心领域代码不依赖 Chrome、终端和具体 Agent；
- 外部 Agent 能力通过 capability 显式表达；
- artifact 写入成功与 Agent 启动成功分别记录；
- 所有代码注释使用中文，重点解释业务背景、设计理由、边界和风险，不复述代码；
- 对安全、并发、跨平台和兼容性分支补充必要注释和测试；
- 重大设计变更写入 `docs/adr/`，并更新本文中的最终结论。
