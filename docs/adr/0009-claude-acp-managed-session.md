# ADR 0009：本地 Claude 默认通过 ACP 托管

## 状态

已被 ADR 0026 取代，2026-07-15。ACP Managed Session 延期到 V0.2 之后评估。

## 背景

早期设计计划在固定工作区打开可见终端，启动 Claude Code CLI 并自动注入首条消息。引入本地 `agentferryd` 后，系统已经具备持有长期会话和结构化事件的运行位置。

用户希望从浏览器完成交接，并在浏览器中查看任务状态和处理必要的权限审批，不需要打开或接管终端。Claude 已有基于 Agent SDK 的 ACP adapter，可以通过 JSON-RPC 创建会话、提交 Prompt 和接收结构化事件。

## 决策

V0.1 中本地 Claude 目标默认使用 ACP Managed Session：

1. `agentferryd` 以目标 Workspace 为工作目录启动 Claude ACP adapter；
2. daemon 创建 ACP Session 并提交捕获内容与有效 Prompt；
3. daemon 持有会话，记录状态、事件和权限请求；
4. Chrome 扩展展示任务状态，并承载必要的权限审批；
5. 默认流程不打开 Terminal.app、iTerm2、Warp 或 Claude Code TUI。

Workspace 仍然是本地 Claude 目标的必填配置，因为它定义 Agent 的文件操作边界。终端类型不再是 V0.1 配置项。

Claude ACP adapter 通过 Agent SDK 工作，不能假设其认证、订阅计费和所有行为与用户现有 `claude` CLI 完全相同。`aferry doctor` 必须验证 adapter 安装、登录状态和基本会话能力。

## 备选方案

### 打开可见终端并启动 Claude CLI

不作为默认路径。它无法向扩展提供可靠的结构化状态和权限请求，也让用户离开浏览器完成交接。

### 解析 Claude TUI 输出

不采用。终端控制序列和界面变化不能形成稳定控制协议。

## 后果

- ACP Client 和 Claude adapter 生命周期进入 V0.1；
- 浏览器入口必须有任务状态和权限审批体验；
- daemon 必须在扩展弹窗关闭后继续持有会话；
- 需要验证 Claude ACP adapter 的认证和计费是否满足目标用户；
- Terminal 模式可以作为未来兼容或故障回退，但不再驱动首版架构；
- 原《开发共识》中“V0.1 以 Terminal 模式为主”的结论由本 ADR 取代。
