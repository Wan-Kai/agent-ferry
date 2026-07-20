# Agent Ferry 当前上下文

> 状态：Current
> 事实来源：Cargo workspace、Extension 入口和当前测试
> 范围：当前模块、数据流与主要边界；未来路线以 Draft 设计和 ADR 为补充

## 产品边界

Agent Ferry 是本地优先的 Browser-to-Agent Handoff 工具。Chrome 扩展捕获页面与用户 Prompt，经 Native Messaging Host 和私有 Unix Socket 交给本地 daemon。daemon 可以启动本机 Agent CLI，或通过 Runs API 把任务提交给远程 Hermes。

Ferry 不实现 Agent Loop，不接管宿主 Agent 的模型、认证、Skills 或项目指令。Workspace 是本地 Agent 的启动上下文，不是文件访问沙箱。

## 当前数据流

```text
Chrome Page
  -> Extension capture / prompt / target selection
  -> agentferry-host Native Messaging framing
  -> agentferryd private Unix Socket
  -> Claude / Codex CLI / Codex App / OpenCode / Remote Hermes
  -> normalized events and bounded local task history
```

## 模块地图

- `extension/`：页面提取、Prompt、目标选择、任务提交、历史与详情 UI。
- `agent-ferry-protocol`：Connector 命令、响应、事件、版本与消息上限。
- `agent-ferry-transport`：本地传输工具。
- `agent-ferry-core`：路径、Connector token、Workspace 等通用能力。
- `agent-ferry-{claude,codex,opencode,hermes}`：宿主 Agent 或远程服务适配器。
- `agent-ferry-daemon`：认证、授权、任务分发、事件归一化、历史和组合装配。
- `agent-ferry-host`：Chrome Native Messaging 与 daemon IPC 的薄桥接层。
- `agent-ferry-cli`：安装、诊断和管理入口，也是受限管理命令的组合根。

## 高风险边界

- 浏览器页面内容不可信，不能影响可执行路径、argv、Workspace、连接或 Connector capability。
- Prompt 与捕获正文不得出现在进程 argv、普通日志或调试输出中。
- Connector token、Hermes token 和 Keychain 内容不得进入协议错误、历史或验证证据。
- 任务输出历史有大小和条数上限，目录与文件必须保持仅当前用户可访问。
- 取消和超时必须只终止目标任务的进程组，不能影响其他并发任务。

## 文档入口

- 当前架构：`docs/architecture/overview.md`
- 依赖规则：`docs/architecture/dependency-rules.md`
- 文档生命周期：`docs/documentation-lifecycle.md`
- 本地开发：`docs/runbooks/development.md`
- 真实环境验收：`docs/runbooks/real-environment-acceptance.md`
