# ACP Client 开源复用调研

> 状态：Historical
> 事实来源：2026-07-15 调研
> 范围：当时可用的 ACP Client 方案，不代表当前已实现依赖

调研日期：2026-07-15。

## 结论

Agent Ferry 不需要自行实现 JSON-RPC framing 或 ACP schema。Rust daemon 应直接依赖官方 `agent-client-protocol` Runtime Crate；Claude 侧启动官方 `agentclientprotocol/claude-agent-acp` adapter。会话 UI 可以研究 Zed 等开源客户端的交互和状态划分，但必须按许可证边界独立实现。

## 可直接依赖的组件

### `agent-client-protocol` Rust SDK

- 官方 ACP Rust Runtime SDK；
- Apache-2.0；
- 提供 Client、外部 Agent 进程、Active Session、请求/通知 dispatch 和 session builder；
- 官方示例覆盖初始化、创建 Session 和发送 Prompt；
- 协议兼容性通过 `initialize` 协商的 `protocolVersion` 和 capabilities 判断，不能通过 crate 版本号猜测。

建议作为 `agent-ferry-daemon` 的直接依赖，并在 Ferry 领域层之外增加薄的 ACP adapter，避免业务代码依赖协议原始消息类型。

### `claude-agent-acp`

- ACP 官方组织维护；
- Apache-2.0；
- 基于官方 Claude Agent SDK；
- 已支持上下文、图片、工具调用、权限请求、Follow-up、Edit Review、TODO、终端和 Slash Commands；
- 以 npm 包发布，当前实现为 TypeScript，需要 Node.js 运行时。

建议固定经过验证的版本，不在每次启动时使用不受控的 `latest`。daemon 的 `doctor` 应验证 adapter 版本、初始化、认证和最小 Session 能力。

## 可参考但不作为核心依赖的项目

### Zed `acp_thread`

Zed 是成熟的 ACP Client，可以参考 thread、turn、tool call、permission 和 session update 的 UI 状态建模。但相关 crate 标注 GPL，除非 Agent Ferry 最终选择兼容许可证，否则不能直接搬代码。即使许可证兼容，也应避免复制整个编辑器领域模型。

### `acpx`

`acpx` 是 MIT 的 headless ACP Client，已实现持久 Session、多 Agent resolution 和命令行交互，可用于研究 session registry、adapter discovery 和端到端测试方式。其运行时接口仍标注 alpha，且同样依赖 Node，因此不适合作为 Rust daemon 的核心控制层。

### `ACP Kit`

`ACP Kit` 封装了进程启动、认证、初始化、Session、事件归一化和跨平台命令处理，适合 TypeScript 产品。它说明 Ferry 需要显式处理这些边界，但 Agent Ferry 已选择 Rust daemon，直接引入另一套 Node Host 会形成重复控制层。

## 建议架构

```text
Chrome Extension
      │ Ferry IPC events
      ▼
agentferryd
      ├── Ferry Session / Turn / Event Store
      ├── Claude Controller
      │     └── official agent-client-protocol Rust SDK
      │             │ ACP JSON-RPC over stdio
      │             ▼
      │       pinned claude-agent-acp
      │             ▼
      │       Claude Agent SDK
      └── Hermes Controller
            └── Runs API + SSE
```

Ferry 自己的 Session/Turn/Event 模型用于浏览器重连、历史展示和跨目标统一状态；ACP 原始事件只保留在协议适配边界。不能为了统一 Claude 与 Hermes 而丢弃权限请求、工具调用等目标特有能力。

## 分发风险

`claude-agent-acp` 需要 Node.js。可选方式包括：

1. 要求用户自行安装 Node 和 npm 包：安装成本最低，但破坏一键安装体验；
2. 首次使用时执行 `npx` 下载：体验看似简单，但依赖网络、npm 可用性和可变版本，供应链与可复现性较差；
3. 在 macOS 安装包中携带固定版本 adapter 和私有 Node runtime：安装包更大，但版本、升级和诊断可控。

基于首版只支持 macOS，推荐第三种。系统 Node 不应成为运行时前置条件，用户已有的 Node 环境也不应影响 Ferry 行为。

## 认证与计费注意事项

Claude adapter 基于 Agent SDK，不能假设其认证和计费永远与交互式 Claude Code CLI 相同。Anthropic 在 2026-06-15 暂停了原计划中的计费调整；当前 Agent SDK 和第三方应用仍计入 Claude 订阅用量，但官方明确表示后续还会更新方案。`doctor` 和产品文档必须展示实际认证方式，不能承诺永久沿用某种订阅政策。

## 参考资料

- [ACP 官方仓库](https://github.com/agentclientprotocol/agent-client-protocol)
- [ACP Rust SDK](https://docs.rs/agent-client-protocol/latest/agent_client_protocol/)
- [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp)
- [Zed acp_thread](https://github.com/zed-industries/zed/tree/main/crates/acp_thread)
- [acpx](https://www.npmjs.com/package/acpx)
- [ACP Kit](https://www.npmjs.com/package/@acp-kit/core)
- [Anthropic：Use the Claude Agent SDK with your Claude plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
