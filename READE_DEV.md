# Agent Ferry 开发共识

> 状态：Draft
> 事实来源：产品讨论和早期路线图；当前实现以 `CONTEXT.md` 与 Current 架构文档为准
> 范围：产品方向、设计约束和尚未解决的问题

本文起始于 2026-07-14 的初始共识。已经实现或被替代的行为必须回到 Current 文档与 ADR 核对，不能仅凭本文判断运行时能力。

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

### 3.0 首要场景

V0.1 首先服务于这样一条用户旅程：用户在浏览器中阅读论文、Twitter/X 帖子或技术文档时，希望将当前内容交给 Agent 继续分析。

首要部署拓扑包括：

- Claude Code 运行在用户本机。Agent Ferry 在预先配置的本地工作区中启动一次性 `claude -p` 非交互任务，将捕获内容带入上下文，并在任务完成后展示最终输出；
- Hermes 运行在用户自己的服务器上，并通过 IM Gateway 与用户持续对话。Agent Ferry 需要把浏览器捕获内容直接提交给该 Hermes profile；Hermes 使用自己的本地文件能力决定是否保存、保存到哪里以及如何建立后续召回所需的索引。

这里的关键价值不是单纯收藏网页，而是免去用户复制正文、切换终端、进入固定目录、启动 CLI 和手工组织上下文的重复操作。

### 3.1 初次安装

理想体验：

```text
安装 Agent Ferry
→ 注册 Chrome Native Messaging Host
→ 安装并启动本地 agentferryd 服务
→ 安装或引导安装 Chrome 扩展
→ 展示当前支持、已检测和需配置的 Agent
→ Claude Code 缺失时给出官方安装指引和 `aferry agent doctor` 复检命令
→ 创建一个或多个工作区配置
```

需要接受的浏览器约束：普通 CLI 不能在所有用户环境中静默安装 Chrome 扩展。正式分发优先考虑 Chrome Web Store；开发版可以使用 unpacked extension。`aferry setup` 可以自动注册 Native Messaging Host，并负责检查或引导扩展安装。

### 3.2 日常使用

```text
浏览网页
→ 打开扩展
→ 可选选择 Prompt 模板并编辑最终 Prompt
→ 选择工作区
→ 选择 Agent
→ 页面内容生成 daemon 管理的临时 Markdown Artifact
→ 在目标工作区启动 Agent 并自动提交首条消息
```

用户点击交接按钮后，系统应自动创建一次性任务并提交包含用户目标和捕获内容引用的消息，让 Agent 立即开始工作。V0.1 不允许中途追加消息或处理权限请求；本地 Claude 使用明确标识的 `unrestricted_host` 模式，以当前操作系统用户权限自主执行，用户只能等待最终结果或取消整个任务。

一次 Handoff 只选择一个目标：本地 Claude 或云端 Hermes。需要同时发送时，用户分别创建两个独立任务；V0.1 不提供多目标广播、结果聚合或部分失败处理。

用户可以在浏览器扩展中配置可复用的 Prompt 模板，并在交接时按需选择。选择模板不是交接的必填条件。未选择模板时，系统将内置的最小默认 Prompt 填入最终 Prompt 编辑框，保证用户可以一键交接。

扩展只提供一个“最终 Prompt”编辑框。选择模板后将解析结果填入该编辑框，用户可以在发送前修改；修改只影响本次交接，不反向覆盖模板。系统不能在编辑框内容之外追加用户不可见的任务指令。

### 3.3 多工作区

工作区由用户创建和选择。一个用户可以配置多个工作区，例如：

- `~/kb`：知识整理；
- `~/projects/product-a`：产品 A 开发；
- `~/writing`：写作。

Workspace 只用于本地 Agent，定义其启动目录和项目上下文。V0.1 的 `unrestricted_host` 不是沙箱，不能把 Workspace 视为文件访问安全边界；Claude 可以访问当前系统用户有权访问的其他路径。不同 Workspace 可以配置允许使用的本地 Agent和默认 Agent。云端 Hermes 使用独立的 Hermes Connection，不向用户暴露服务器工作区路径。

## 4. 首批支持的 Agent

首批目标范围：

- Claude Code；
- 云端 Hermes。

Codex、OpenCode、Pi 和本地 Hermes 在首版纵向闭环稳定后再接入。

Agent Ferry 不重新实现这些 Agent，也不接管它们的模型配置、订阅、认证、Skills、MCP 或项目指令。V0.1 直接调用用户已有的 Claude Code CLI 并沿用其配置和认证；后续 Managed 模式尽可能保持这一原则。

## 5. 产品边界

### 5.1 核心价值

Agent Ferry 聚焦三件事：

1. 从浏览器可靠地捕获有价值的内容；
2. 将内容物化为可检查的临时 artifact，并交给正确的本地会话；
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

## 6. 控制路径

社区实践表明，控制 Coding Agent 主要分成两条路线。Agent Ferry 应同时保留两条路径，不强行用一个接口掩盖能力差异。

### 6.1 Print Task 模式

Agent Ferry 在指定 Workspace 中启动用户已有的 Claude Code CLI，使用 `-p` 执行一次性非交互任务，通过 `stream-json` 读取结构化进度与最终结果。

特点：

- 沿用用户已有订阅、登录、CLAUDE.md、Skills、hooks 和项目配置；
- 用户提交一次任务，中途不能继续对话或审批，完成后返回最终输出；
- 每次交接创建新的 task/session UUID；不使用 `--continue` 或 `--resume`，同一 Workspace 的多个任务也可以并行；
- daemon 可以展示运行、失败、完成和取消状态，但不持有可交互会话；
- 回复和最终答案只实时转发到当前浏览器界面，不写入结果历史，也不提供重新打开后的重放；
- 不传 `--no-session-persistence`，允许 Claude Code 保持原生默认会话记录；Ferry 不读取、展示或依赖该记录；
- 浏览器界面关闭不取消任务；daemon 继续排空 Claude 输出但在没有订阅者时直接丢弃，attach、补发和 Side Panel 延期到 ACP 阶段；
- V0.1 使用 `unrestricted_host`，不向 Claude 添加工具或路径限制；
- Workspace 是启动目录而不是沙箱，真实权限等于当前操作系统用户权限以及 Claude Code 自身仍然生效的硬约束。

这是 V0.1 中本地 Claude 的唯一运行路径。

### 6.2 Terminal 模式

Agent Ferry 在指定工作区中打开一个可见、可交互的原生终端，并启动用户选择的 Agent CLI。

特点：

- 保留 Agent 原生 TUI；
- 沿用用户已有订阅、登录和配置；
- 用户可以立即接管；
- 适配范围最广；
- Ferry 只负责启动，不承诺知道 Agent 的结构化状态。

这不是 V0.1 中本地 Claude 的路径，只作为未来兼容或故障回退候选。

### 6.3 Managed 模式

Agent Ferry 通过 ACP 或 Agent 原生 RPC 持有会话，负责消息、事件、权限审批、取消和恢复。

特点：

- 可以继续向同一个会话发送内容；
- 可以展示结构化进度和错误；
- 可以处理 Agent 的权限请求和澄清问题；
- 需要 Ferry 自己提供相应 UI；
- 需要常驻 `agentferryd` 持有子进程或连接。

Managed 模式延期到 V0.2 之后。届时统一层优先选择 ACP；只有 ACP 无法提供关键能力时，才评估 Agent 专用 RPC Controller。ACP 的消息续写、权限审批、恢复与 adapter 安装不进入 V0.1。

## 7. 内容面与控制面

架构必须明确区分两个平面。

### 7.1 Content Plane

负责：

- 页面标题、URL、正文、选区和元数据；
- HTML 到 Markdown 的转换；
- 临时 artifact 命名、隔离、落盘和清理；
- 通过 SSH 写入远程工作区；
- 后续可能支持图片、PDF 和附件。

Markdown 文件是最通用、最持久的内容接口。任何 Agent 都能读取文件，用户也可以在 Agent 运行前后检查和修改内容。

### 7.2 Control Plane

负责：

- 检测 Agent 是否安装；
- 启动一次性任务；
- 读取结构化输出、获取最终结果和取消任务；
- 未来的继续发送、事件订阅、审批、恢复和 attach；
- Terminal、ACP、原生 RPC 和远程连接的差异。

文件 inbox 不是控制协议。文件写入成功不等于 Agent 已经启动或已经读取文件。

## 8. V0.1 架构

```text
Chrome Extension
      │ Native Messaging
      ▼
agentferry-host（薄桥接层）
      │ 本地 IPC
      ▼
agentferryd（本机交接中枢）
      ├── 配置、凭据引用与目标能力
      ├── 任务状态与交接历史
      ├── Direct / SSH Tunnel 与连接生命周期
      └── agent-ferry-core
      ├── 提取请求校验
      ├── 工作区解析
      ├── 临时 Markdown artifact 管理
      ├── Agent 检测与启动参数构造
      └── 目标适配器
          ├── 本地 Print Task → Claude Code CLI
          ├── 本地 Terminal（未来兼容或故障回退）
          └── 远程连接 → Hermes Gateway
```

### 8.1 为什么 V0.1 引入 Daemon

初始设计认为 V0.1 只负责写入内容和启动一个用户可见的终端，因此可以依赖 Chrome 按需启动 Native Messaging Host。现在的首要范围还包括长期运行的远程 Hermes Gateway，并需要为多个入口共享目标连接、任务状态和交接历史。

因此 `agentferryd` 在 V0.1 中作为本机交接中枢。Native Messaging Host 只做协议桥接；daemon 不实现 Agent Loop，也不接管目标 Agent 的配置和记忆。

V0.1 只允许 Chrome Native Host 通过当前用户私有的 Unix Domain Socket 连接 daemon，不开放本地 HTTP、WebSocket 或 TCP。内部从第一版起拆分 Transport、Connector Authentication、Principal 与业务命令；未来其他本地、网页或云端连接方必须实现独立鉴权、撤销和重放保护，不能沿用 Native Host 的本机信任。

已认证连接方仍按 capability 最小授权。Chrome Native Host 只允许提交捕获、读取目标、创建/查看/取消任务；adapter 管理、Hermes 凭据、Agent 可执行路径及 daemon 管理均由 CLI 专属 Principal 处理。这里限制的是“扩展能调用哪些 Ferry 命令”，不是限制 Claude 子进程的工具权限；权限必须由 daemon 校验，不能只依靠扩展隐藏管理按钮。

### 8.2 Daemon 的边界

- 负责配置、任务、历史、凭据引用、本地 IPC、Direct / SSH Tunnel 和目标连接；
- 不实现 Agent Loop；
- V0.1 只接受 Chrome 扩展入口，不实现网页或云端入口；
- V0.1 不解析本地 Agent TUI 状态；Claude 的结构化状态来自 Print Mode `stream-json`；
- 未来网页和云端入口必须使用认证连接，daemon 不默认监听公开网络接口；
- ACP/RPC 长连接、消息续写、权限审批和恢复延期到 Managed Session 阶段；V0.1 只保留终止一次性子进程的任务取消。

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
├── effective_prompt
├── target
│   ├── local_claude
│   │   └── workspace_id
│   └── cloud_hermes
│       └── connection_id
```

`target` 是单个带类型判别的值，不是列表。未来若需要多目标执行，应在 Handoff 之上增加显式的 Batch/Workflow，而不是让一个任务同时拥有多个 Controller。

工作区：

```text
Workspace
├── id
├── name
├── root
├── allowed_agents
└── default_agent
```

本地 Claude 目标必须选择本地 Workspace，并固定使用一次性 `unrestricted_host` Print Task；云端 Hermes 目标只选择连接实例，不展示或提交 Workspace 和 Execution Mode 字段。

Agent Controller 应声明能力，而不是假设所有 Agent 功能相同：

```text
detect
start_task
stream_events
cancel
get_result
```

扩展只展示目标 Agent 和当前模式真正支持的操作。

## 10. Agent 接入策略

| Agent | V0.1 | Managed 首选 | 必要时的专用接口 |
|---|---|---|---|
| Claude Code | `claude -p` Print Task | 后续 ACP adapter | `stream-json` / Agent SDK |
| Codex | 原生终端 | ACP adapter | Codex app-server |
| OpenCode | 原生终端 | 原生 ACP | HTTP/OpenAPI/SSE |
| Pi | 原生终端 | ACP 或 Pi RPC | JSONL RPC |
| 云端 Hermes | Runs API（Direct 优先，SSH Tunnel 回退） | HTTP + SSE | Hermes API Server |
| 本地 Hermes | V0.1 不支持 | 原生 ACP | TUI Gateway |

### 10.1 为什么不优先使用 PTY

PTY 可以托管真实 CLI，并允许 Ferry 注入键盘输入和展示终端画面，但无法稳定理解不同 TUI 的语义。Agent 更新、终端控制序列或权限界面变化都可能破坏控制逻辑。

PTY 可以作为未来可选的 `Managed Terminal` 后端，但不作为 V0.1 的基础，也不通过屏幕文本解析来假装获得可靠的结构化状态。

### 10.2 为什么 ACP 不是 MCP

- ACP 的方向是 Client/Editor 控制 Agent session；
- MCP 的方向是 Agent/Host 获取工具、资源和提示模板。

因此“不用 MCP 中转”的早期决策仍然成立。未来使用 ACP 管理 Agent，不与该决策冲突。

## 11. 本地与远程工作区

### 11.1 本地

- 在操作系统提供的临时目录中创建按 Handoff 隔离的 Markdown artifact，不硬编码 `/temp` 或 `/tmp`；
- 使用结构化 argv 启动 Agent；
- 避免拼接未经转义的 shell 字符串；
- 将有效 Prompt 写入 Claude 子进程 stdin 后关闭输入，不把 Prompt 放入可能被进程列表读取的 argv；完整捕获正文仍通过 Artifact 路径传递；
- Print Task 的 Prompt 展示有效 Prompt、来源信息和 artifact 绝对路径，并明确要求 Claude 读取文件；
- V0.1 以 `--permission-mode bypassPermissions` 启动 Claude，不设置 Ferry 工具白名单；Workspace 只提供 cwd，不限制 Claude 访问当前系统用户可访问的其他路径；
- daemon 负责限制 artifact 自身的文件权限、容量和过期清理；Artifact 在任务结束后默认保留 24 小时，运行中的任务不清理。

### 11.2 云端 Hermes

V0.1：

- 通过 Hermes API Server 的 Runs API，把捕获内容和有效 Prompt 提交给现有 IM Gateway 使用的同一 profile；
- 将最终 Prompt、来源信息和完整 Markdown 直接放入单次 Runs API `input`，不静默截断，也不在首版实现分块上传；
- 优先直接连接用户已有的 Tailscale、WireGuard、可信局域网或 HTTPS Endpoint；只有 SSH 可达时，由 daemon 复用 `~/.ssh/config` 建立 Tunnel；
- 两种传输都使用 Hermes Bearer Token，凭据值保存在 macOS Keychain；
- 连接建立后先读取 `/v1/capabilities`，按服务器实际能力启用 SSE 状态、审批和取消；
- Hermes 根据自己的规则决定是否持久化、保存路径、摘要和索引方式；
- Hermes 声明 SSE 能力时只向当前浏览器界面实时转发状态和输出，Ferry 不保存结果；界面关闭不取消远程 Run，也不提供重新 attach 或补发；
- Agent Ferry 不通过 SSH/SFTP 管理远程文件，不建立远程 inbox，不直接写 Hermes 的 `MEMORY.md`；
- 请求过大或模型上下文不足时明确报告失败并保留本地捕获结果；分块协议根据真实失败样本再设计；
- 远程交接结果由 Hermes 返回。若实例没有可用的本地文件工具，只能报告分析完成，不能报告持久化完成。

V0.1 不要求用户启用外部 memory provider。后续 IM 中的召回依赖 Hermes 自己对服务器本地文件、会话记录和记忆的管理。

不直接把未经保护的本地控制端口暴露到公网。OAuth/OIDC 公网登录和 Agent Ferry 自建云中继不进入 V0.1。

## 12. Chrome 扩展

技术选型：

- WXT；
- React；
- TypeScript；
- Chrome Manifest V3；
- Native Messaging 与 Rust Host 通信。

扩展职责：

- 提取当前页面标题、URL、正文和用户选区；
- 交接浏览器当前会话能够读取的实际内容，不能用 URL 代替正文并依赖 Agent 重新抓取；
- 转换或预处理为 Markdown；
- 收集 Prompt 模板选择、最终 Prompt、Workspace 和目标 Agent；
- 管理并选择用户配置的 Prompt 模板；
- 允许用户编辑并确认最终将发送给 Agent 的完整 Prompt；
- 展示写入和启动结果；
- 不直接监听未经认证的 localhost HTTP 端口；
- 不持有 Agent API Key。

V0.1 使用 Defuddle 作为核心内容提取依赖。普通站点先通过有明确上限的自动滚动尽量触发懒加载，恢复用户位置后再使用通用解析；Twitter/X 和 YouTube 优先复用 Defuddle 的专用 Extractor；arXiv HTML 先走通用解析；arXiv PDF 使用独立的 PDF 获取与解析链路。

只选择性参考 Obsidian Web Clipper 的浏览器集成代码，不复制整个项目，也不引入其 Vault、笔记模板、Highlight、Reader 或 Interpreter 领域模型。

捕获结果应保留来源 URL、捕获时间和捕获方式，并在无法获得完整内容时明确标记，而不是静默生成看似完整的 artifact。

Node.js 只用于扩展开发和构建。正式用户安装预构建扩展，不需要 Node.js 运行环境。

## 13. Rust 工程

选择 Rust 的原因：

- 项目本身是学习 Rust 的实践载体；
- 适合构建无运行时依赖的跨平台 CLI、Native Host 和本地 Daemon；
- 对进程、IPC、文件系统和并发控制有良好支持；
- 可以从同一代码库发布 macOS、Linux 和 Windows 二进制。

初始 workspace：

```text
crates/
  agent-ferry-core/      不依赖 UI 的领域模型和用例
  agent-ferry-protocol/  Native Messaging 请求与响应
  agent-ferry-cli/       aferry setup/config/doctor 等命令
  agent-ferry-host/      Chrome Native Messaging Host
  agent-ferry-daemon/    agentferryd 本地交接中枢
```

Daemon 需要同时处理本地 IPC、任务状态和远程连接，已经存在明确并发需求。异步运行时的具体选择仍需结合 IPC 与 HTTP/SSH 客户端方案评估，不能只因为生态惯例默认引入全部功能。

V0.1 不包含 ACP Client 或 Claude ACP adapter。daemon 直接启动用户已有的 Claude Code，并解析其 `stream-json` 输出。后续进入 Managed Session 阶段时，ACP Client 优先直接依赖官方 Apache-2.0 `agent-client-protocol` Rust Runtime SDK，不自行实现 JSON-RPC framing 和协议 schema；adapter 和私有 runtime 仍不得进入 Core。

## 14. 安全约束

网页内容是不可信输入，可能包含 prompt injection、恶意文件名、超长内容和终端控制字符。

最低安全要求：

- 页面正文只作为用户 artifact，不作为 Ferry 自己的控制指令；
- 文件名必须清洗并限制长度；
- Ferry 自己创建和管理的 artifact、配置与缓存路径必须经过规范化和权限校验；`unrestricted_host` 下不能承诺 Claude 的文件访问被限制在 Workspace；
- 不允许网页内容影响可执行文件、argv、环境变量或工作区选择；
- 启动命令使用结构化参数，不拼接 shell 命令字符串；
- Native Messaging manifest 只允许正式扩展 ID；
- 日志不得记录 token、密码或完整隐私内容；
- 远程连接默认走 SSH，不公开裸控制端口；
- V0.1 本地 Claude 固定以当前系统用户权限执行且中途没有审批，不增加独立 execution mode 配置、启用确认或逐任务确认；目标诊断和产品文档必须准确说明这一临时边界；
- 后续 Managed 或 sandbox 模式必须重新定义权限和隔离边界，不能把 V0.1 的不受限行为静默沿用。

## 15. 发布与安装

用户下载预编译产物，不要求安装 Rust、Node.js、npm、Bun 或 Python。

发布采用轻量 Core + 后续按需 Agent Pack。Core 只包含 daemon、Native Host、CLI、系统注册和所有用户都需要的基础能力。第三方宿主 Agent 由用户自行安装、认证和升级，Ferry 只检测并给出官方指引。V0.1 的 Claude Print Task 不下载 adapter 或 runtime；以后支持 ACP、Codex、Pi 等目标时，相关 adapter/runtime 才按需安装，不能因为新增 Agent 扩大所有用户的 Core 下载。

`aferry setup` 必须在下载 Agent Pack 前展示组件、固定版本、来源、下载体积和磁盘占用。禁止运行时隐式下载 `latest`。共享 runtime 只安装一份，并由 Ferry 统一升级和回滚。

V0.1 使用 `aferry agent list/enable/disable/doctor` 管理 Claude 目标，使用 `aferry connection add/list/doctor` 管理云端 Hermes。`aferry adapter list/install/update/remove` 作为后续 ACP 阶段的命令空间保留，但首版没有需要安装的 adapter。`aferry setup` 只负责检查、展示状态和给出下一条命令，浏览器扩展和 daemon 运行任务时不得静默安装组件。

Ferry 不读取或保存 Claude Code 等第三方宿主 Agent 的凭据。宿主是否安装通过可执行文件和版本判断，认证状态通过受限的 Print Mode doctor 调用判断；未认证时提示用户使用宿主自己的登录流程，再运行 `aferry agent doctor` 复检。云端 Hermes 的 Connection token 属于 Ferry 显式配置，单独保存到系统 Keychain。

宿主 Agent 只检测到一个兼容可执行文件时，Ferry 自动绑定其绝对路径；检测到多个时进入 `needs_selection`，要求用户通过 `aferry agent enable <id> --command <absolute-path>` 选择。运行配置不接受 shell 字符串或附加参数，已绑定目标也不会因 `PATH` 变化静默切换。

每次新建本地 Agent 任务前检查已绑定可执行文件的路径、权限和文件身份。文件未变化时复用上次兼容性结论；文件发生变化时重新检查宿主版本和 Print Mode 所需 flags。不兼容时阻止任务并提示用户升级 Claude Code，Ferry 不负责升级宿主。

首个可用版本只正式支持 macOS 客户端。Linux 服务器上的远程 Hermes 仍在首版范围内，但 Linux 和 Windows 桌面客户端在 macOS 纵向闭环稳定后再补充。

计划的发布渠道：

1. GitHub Releases：所有平台的基础分发渠道；
2. Homebrew：macOS/Linux 的首选安装方式；
3. WinGet：Windows 稳定后加入；
4. `cargo install`：主要面向 Rust 开发者和源码安装。

计划发布目标：

- 首发：macOS Apple Silicon；
- 后续按需：macOS Intel、Linux x86_64、Linux ARM64、Windows x86_64 MSVC。

Windows MSVC 正式产物使用 GitHub Actions Windows Runner 原生构建和测试。macOS 到 MSVC 的交叉编译可以用于实验，但不作为唯一正式发布链路。

## 16. 路线图

### V0.1：可靠交接

- macOS 客户端纵向闭环；
- Chrome 扩展捕获页面；
- 本地 `agentferryd` 交接中枢；
- daemon 管理的临时 Markdown artifact；
- 多工作区；
- Claude Code Print Mode 检测、一次性任务和 `stream-json` 结构化状态；
- 本地 Claude 使用显式标记的 `unrestricted_host`，中途不审批、不续写；
- 浏览器显示运行状态与最终输出，并允许取消整个任务；取消会终止 Claude 进程组，部分输出不标记为最终答案；
- 首版不保存回复正文或最终答案，不提供任务历史、完成通知或重新打开查看；
- 本地 Claude 任务默认最长运行 60 分钟，超时终止进程组并标记为 `timed_out`；首版浏览器不提供超时配置；
- 每次交接直接启动独立 Claude 对话，不按 Workspace 排队；Ferry 不协调同一 Workspace 中并发任务的文件和 Git 修改；
- 连接现有远程 Hermes profile，由 Hermes 自主管理持久化并供 IM 会话召回；
- `aferry setup`、`aferry doctor`、`aferry config`；
- Native Messaging Host 注册；
- 轻量 Core；V0.1 不安装 Claude adapter；
- GitHub Releases 和 Homebrew 安装。

### V0.2：扩大托管能力

- 设计并实现与 `unrestricted_host` 并列的 sandbox 执行模式；
- 评估 Claude ACP Managed Session、消息续写、权限审批和恢复；
- Codex、OpenCode、Pi 和本地 Hermes 的托管接入；
- 更完整的流式状态；
- 更细粒度的权限策略；
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
- Windows Terminal 和 Linux terminal launcher 的优先级；
- Handoff 历史使用纯文件、JSON 还是 SQLite；
- V0.2 的 ACP session 是否由扩展直接呈现，还是增加独立桌面/Web UI。

## 19. 开发原则

- 先完成纵向闭环，再扩充 Agent 数量和高级能力；
- 始终以用户下载和安装尽量轻量为方案选型约束；功能相当时优先更小的 Core、更少的常驻进程和更低的磁盘占用；
- 新增 Agent 默认不能扩大 Core；重型 runtime、adapter 和可选解析能力必须按需安装；
- 核心领域代码不依赖 Chrome、终端和具体 Agent；
- 外部 Agent 能力通过 capability 显式表达；
- artifact 写入成功与 Agent 启动成功分别记录；
- 所有代码注释使用中文，重点解释业务背景、设计理由、边界和风险，不复述代码；
- 对安全、并发、跨平台和兼容性分支补充必要注释和测试；
- 重大设计变更写入 `docs/adr/`，并更新本文中的最终结论。
