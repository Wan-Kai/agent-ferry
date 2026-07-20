# 多 Agent ACP 技术栈建议

> 状态：Historical
> 事实来源：2026-07-15 调研
> 范围：未来 ACP 技术栈候选，不代表当前实现

调研日期：2026-07-15。

## 结论

推荐采用混合技术栈：

- Chrome Extension：TypeScript + React + WXT；
- 本地中枢 `agentferryd`：Rust + Tokio；
- ACP Client：官方 `agent-client-protocol` Rust SDK；
- Agent 接入：优先启动各项目的原生 ACP 命令或官方 adapter 子进程；
- 云端 Hermes：HTTP Runs API + SSE，不强行套用 stdio ACP；
- 本地持久化：SQLite 事件存储；
- 密钥：macOS Keychain；
- 本地通信：Native Messaging Bridge + Unix Domain Socket。

核心原则是统一协议和领域事件，而不是统一每个 Agent 的实现语言或运行时。

## 为什么不建议全 TypeScript

ACP 的官方 TypeScript SDK 和多个 adapter 确实让纯 Node Host 很容易起步，但 Agent Ferry 还需要长期 daemon、进程监管、Native Messaging、SSH Tunnel、临时文件权限、Keychain、安装注册和未来跨平台分发。这些工作由 Rust 承担更稳定，也更符合项目当前工程结构。

Node.js 只作为部分 adapter 的私有运行时存在，不成为系统中枢，也不暴露给业务领域层。

## 为什么不建议全部重写成 Rust

Claude、Codex、OpenCode、Pi 和 Hermes 的原生实现与 adapter 更新频繁。如果 Ferry 把每个桥接层都重写成 Rust，就需要长期追踪多个私有协议和事件格式，失去 ACP 的主要价值。

daemon 应只维护一个 ACP Client，实现：

- `initialize` 与协议版本协商；
- capabilities 检测；
- Session 创建、加载和继续发送；
- 流式 update；
- permission；
- cancel；
- 进程生命周期、超时和错误映射。

Agent 差异放在启动描述和 capability 中，不通过大量 `match agent_name` 分支进入核心状态机。

## Agent 接入矩阵

| Agent | 推荐入口 | 运行时归属 | 首选策略 |
|---|---|---|---|
| Claude Code | `claude-agent-acp` | 固定版本 Node adapter + Claude Agent SDK | Ferry 携带私有 Node runtime 和固定 adapter |
| Codex | `@agentclientprotocol/codex-acp` | 官方 npm adapter，内部连接 Codex app-server | 优先使用固定 adapter；验证是否可采用其 standalone bundle |
| OpenCode | `opencode acp` | OpenCode 自身 | 调用用户已安装的原生命令，不再加一层 adapter |
| Pi | `pi-acp` 或基于 `pi --mode rpc` 的 adapter | 当前主要为社区 Node/Go adapter | 延后接入，先通过 registry 和实测选择成熟实现 |
| 本地 Hermes | `hermes acp` | Hermes 自带 Python 环境 | 调用用户已安装的原生命令 |
| 云端 Hermes | Runs API + SSE | 服务器 Hermes Gateway | 保持 HTTP Controller，不经本地 stdio ACP |

OpenCode 与 Hermes 已原生提供 ACP 命令。Claude 和 Codex 有 ACP 官方组织维护的 adapter。Pi 当前 ACP Registry 中的实现是社区 `pi-acp`，成熟度和能力需要单独验证，不能与前四者视为同等稳定。

## 推荐进程架构

```text
Chrome Extension (TypeScript/WXT)
             │
      Native Messaging
             ▼
agentferry-host (Rust, framing only)
             │ Unix Domain Socket
             ▼
agentferryd (Rust/Tokio)
├── Session / Turn / Event Store (SQLite)
├── ACP Runtime (official Rust SDK)
│   ├── claude-agent-acp   [private Node runtime]
│   ├── codex-acp          [pinned adapter/bundle]
│   ├── opencode acp       [installed native command]
│   ├── pi-acp             [community adapter]
│   └── hermes acp         [installed Hermes command]
└── Remote Hermes Controller
    └── HTTP Runs API + SSE
```

## Rust 侧建议组件

### ACP 与并发

- `agent-client-protocol`：官方 ACP Runtime；
- Tokio：子进程、IPC、定时器和异步任务；
- `tokio-util` CancellationToken：Turn 和进程取消传播；
- `serde` / `serde_json`：Ferry IPC 和持久化 payload。

### HTTP 与远程连接

- `reqwest`：Hermes REST；
- SSE parser 使用经过维护的库或基于 `reqwest` byte stream 的小型适配层；
- SSH Tunnel 优先监管系统 `/usr/bin/ssh` 子进程，复用 OpenSSH 配置与 host key 语义，不在首版自行实现 SSH 协议栈。

### 存储

- SQLite + WAL；
- 可选 `sqlx` 或 `rusqlite`，根据 daemon 的异步边界选择；
- 持久化 Ferry 归一化事件和必要的 ACP 原始 payload，前者供 UI，后者供诊断和兼容升级；
- 事件带单调递增 sequence，使扩展重连后可以从游标补齐。

### 系统集成

- Keychain：保存 Hermes API Key 等 Ferry 自己拥有的凭据；
- LaunchAgent：管理 `agentferryd`；
- Unix Domain Socket：host 到 daemon；
- adapter 的 Agent 登录仍由各 Agent 自己管理，Ferry 不复制其 credential store。

## Ferry 内部事件模型

不要把 ACP `session/update` 原样暴露成扩展的长期 API。建议归一化为稳定的 Ferry 事件：

```text
SessionCreated
TurnStarted
MessageDelta
MessageCompleted
ToolCallStarted
ToolCallUpdated
ToolCallCompleted
PermissionRequested
PermissionResolved
TurnCompleted
TurnFailed
SessionUnavailable
```

每个事件保留：`event_id`、`session_id`、`turn_id`、`sequence`、`occurred_at`、`source_protocol` 和可选 `raw_payload`。目标特有内容通过扩展 payload 保留，不能为了统一而丢失。

## 分发策略

建议将“ACP Host”和“Agent 安装”分开：

1. Ferry 始终携带 Rust ACP Client；
2. 对 Claude 这类必须 adapter 的首要目标，携带固定 adapter 与私有 Node runtime；
3. 对 OpenCode、本地 Hermes 等原生 ACP Agent，检测用户现有命令和版本；
4. 对 Codex，优先评估官方 adapter 的 standalone bundle，无法满足时再随 Ferry 的私有 Node runtime 分发；
5. 对 Pi 等社区 adapter，默认不自动下载 `latest`，只安装经过 Ferry 兼容矩阵验证的版本；
6. `aferry doctor` 对每个 Agent 实际执行无 token 消耗的 `initialize` / capability probe。

## 测试策略

- 使用录制的 `initialize`、capabilities 和 Session 事件 fixture 做确定性测试；
- 为每个 adapter 建立 opt-in 本机 smoke test，不在普通 CI 消耗模型 token；
- 测试 adapter 无输出、stdout 污染、JSON 损坏、权限超时、进程崩溃和版本不兼容；
- capability 驱动 UI，不用 Agent 名称推断功能；
- 固定 adapter 版本，并在升级 PR 中自动生成能力矩阵差异。

## 参考资料

- [ACP Rust SDK](https://docs.rs/agent-client-protocol/latest/agent_client_protocol/)
- [ACP Agent Registry](https://github.com/agentclientprotocol/registry)
- [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp)
- [Codex ACP Registry Entry](https://github.com/agentclientprotocol/registry/blob/main/codex-acp/agent.json)
- [OpenCode ACP](https://dev.opencode.ai/docs/acp/)
- [Pi ACP Registry Entry](https://github.com/agentclientprotocol/registry/blob/main/pi-acp/agent.json)
- [Hermes Programmatic Integration](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)
