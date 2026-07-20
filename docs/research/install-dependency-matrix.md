# 用户安装依赖矩阵

> 状态：Historical
> 事实来源：2026-07-15 调研
> 范围：早期安装依赖决策背景，当前发布契约以 Current 文档为准

调研日期：2026-07-15。

## 目标体验

普通用户不应为了安装 Agent Ferry 手动安装 Rust、Cargo、Node.js、npm、Bun 或 Python。Core 必须保持轻量，只负责本机 daemon、Native Messaging Host 和系统注册；需要额外 runtime 的 Agent 通过固定版本、签名和校验的 Agent Pack 按需安装。

## 所有用户都需要

1. macOS 首发支持版本；
2. Chrome；
3. Agent Ferry 核心安装包；
4. Agent Ferry Chrome 扩展。

核心安装包包含：

- `agentferryd`；
- `agentferry-host`；
- `aferry` CLI；
- LaunchAgent 配置；
- Native Messaging Host manifest；
- SQLite 支持和数据库 migration；
- macOS Keychain 集成；
- 官方 ACP Rust Client SDK 编译产物。

用户不需要单独安装数据库、OpenSSL 或 Rust runtime。

## 按目标增加的依赖

### 只使用云端 Hermes

本机不需要 Node、Python 或 Hermes CLI。

服务器需要：

- 已安装并正常运行的 Hermes；
- Hermes API Server 已启用；
- `API_SERVER_KEY`；
- Direct 模式下可达的 Tailscale/WireGuard/可信网络/HTTPS 地址，或 SSH Tunnel 模式下可用的 SSH 登录。

Direct 模式不要求本机安装额外 SSH 工具；SSH Tunnel 使用 macOS 自带 `/usr/bin/ssh`。

### 本地 Claude

推荐由 Ferry 安装 Claude Agent Pack，包含：

- 固定版本 `claude-agent-acp`；
- 私有 Node.js 22+ runtime；
- 完整、固定的 `@anthropic-ai/claude-agent-sdk` 依赖与当前平台二进制；
- 版本和完整性 manifest。

用户无需全局安装 Node、npm 或 `claude-agent-acp`。是否仍要求单独安装 Claude Code CLI，需要通过打包 smoke test 验证；Agent SDK 包含平台 CLI 解析能力，但不能在未验证的情况下承诺完全独立于用户现有 Claude Code 安装。

用户仍需要完成 Claude 认证，并拥有相应订阅或 API 计费能力。认证和计费不属于 Ferry 安装依赖。

### Codex

官方 `codex-acp` 已提供 Bun compile 的多平台 standalone bundle 构建方式，并依赖兼容的 `@openai/codex`。优先将验证过的 standalone adapter 放入 Codex Agent Pack。

目标体验是不要求用户全局安装 Node 或 Codex CLI，但用户仍需完成 ChatGPT/API Key 认证。若 standalone bundle 的发布和兼容性不足，则临时复用 Ferry 私有 Node runtime 与固定 npm 依赖。

### OpenCode

OpenCode 原生提供 `opencode acp`。用户需要安装并登录 OpenCode；Ferry 只检测命令路径、版本和 capabilities，不额外安装 adapter。

未来可以增加由 Ferry 管理的 OpenCode 安装，但不应成为首版基础设施。

### Pi

用户需要安装 Pi。当前 ACP Registry 使用社区 `pi-acp` npm adapter，Ferry 应提供固定版本 adapter pack，复用已有私有 Node runtime；不得在运行时调用不固定版本的 `npx latest`。

Pi 的 ACP adapter 尚需兼容性验证，因此不应与 Claude 同时成为首版安装承诺。

### 本地 Hermes

用户需要安装 Hermes，并确保 `hermes acp` 可用。Hermes 自己管理 Python 环境；Ferry 不携带第二套 Hermes 或 Python runtime。

## 推荐的安装产品结构

```text
Agent Ferry Core
├── Rust daemon / host / CLI
├── Chrome Native Messaging 注册
└── Keychain / SQLite / LaunchAgent

Agent Packs（按需）
├── Claude Pack
│   ├── private Node runtime
│   ├── claude-agent-acp
│   └── Claude Agent SDK platform payload
├── Codex Pack
│   └── codex-acp standalone bundle
└── Pi Pack（后续）
    └── pinned pi-acp

External Native Agents
├── opencode acp
└── hermes acp
```

`aferry setup` 应检测目标并显示将安装的组件、版本、体积和来源。用户必须明确选择 Agent Pack，不能因为安装核心 Ferry 就静默下载所有 Agent runtime。

新增 Agent 默认不能改变 Core。若多个 Agent Pack 依赖 Node 等同类 runtime，应由 Ferry 共享一份受控安装，避免重复下载和占用磁盘。

## 首版最小安装组合

### 云端 Hermes 用户

```text
Agent Ferry Core
+ Chrome Extension
+ Hermes Connection 配置
```

### 本地 Claude 用户

```text
Agent Ferry Core
+ Chrome Extension
+ Claude Agent Pack
+ Claude 登录
```

## 仍需实测确认

- `claude-agent-acp` 连同完整 Agent SDK platform payload 打包后，是否可以在没有全局 Claude Code CLI 的干净 macOS 用户中运行和完成认证；
- Codex 官方 standalone adapter 是否完整携带目标平台 Codex binary；
- 私有 Node runtime 与 macOS codesign/notarization 的打包方式；
- Agent Pack 的升级、回滚、签名和供应链 manifest；
- 各 Agent 登录流程能否完全在浏览器扩展中完成，还是需要一次系统浏览器跳转。

## 参考资料

- [Claude Agent ACP package.json](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/package.json)
- [Claude Agent SDK npm package](https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk)
- [Codex ACP package.json](https://github.com/agentclientprotocol/codex-acp/blob/main/package.json)
- [OpenCode ACP](https://dev.opencode.ai/docs/acp/)
- [Pi ACP Registry Entry](https://github.com/agentclientprotocol/registry/blob/main/pi-acp/agent.json)
- [Hermes Programmatic Integration](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)
