# ADR 0007：使用本地 Daemon 作为交接中枢

## 状态

已接受，2026-07-14。

## 背景

早期设计只考虑浏览器触发一次内容落盘并打开本地终端，因此认为 V0.1 不需要常驻 Daemon。讨论后，产品需要同时支持浏览器扩展、本地 Claude Code、长期运行的远程 Hermes Gateway，并为未来网页和云端入口保留连接能力。

这些需求涉及长期目标连接、SSH Tunnel、任务状态、交接历史和多个入口。让每次启动的 Native Messaging Host 独立承担这些职责，会造成连接重复、状态分散和凭据管理困难。

## 决策

V0.1 引入本机常驻进程 `agentferryd`，作为 Agent Ferry 的交接中枢：

```text
Chrome Extension ─ Native Messaging ─ agentferry-host ┐
本地 CLI ─────────────── 本地 IPC ────────────────────┼→ agentferryd
未来网页或云端 ───────── 受认证的连接 ────────────────┘
                                                        │
                                                        ├→ 本地 CLI Agent
                                                        └→ 远程 Hermes Gateway
```

`agentferryd` 负责配置解析、目标能力发现、任务状态、交接历史、凭据引用、SSH Tunnel 和目标适配器生命周期。它不实现 Agent Loop，也不接管 Agent 的模型、认证、skills 或 memory。

`agentferry-host` 保持为薄桥接层，只负责 Chrome Native Messaging framing、扩展来源校验，以及把请求转发给本地 daemon。Chrome 扩展不直接持有 SSH 凭据或 Agent API Key。

V0.1 只有 Chrome 扩展一个交接入口。未来网页和云端入口只保留架构扩展点，不进入首版实现；增加时必须经过独立认证或使用受控的反向连接，不能因为 daemon 位于本机就默认暴露未经认证的 HTTP 端口。

## 备选方案

### 每次交接按需启动 Native Messaging Host

不再采用。它不适合复用远程连接、维护任务状态或支持多个入口。

### 由 Chrome 扩展直接连接所有目标

不采用。扩展不应持有服务器凭据，也难以可靠管理本地进程和 SSH Tunnel。

### 让 Hermes Gateway 充当所有目标的中枢

不采用。Hermes 是一个目标 Agent，不能管理本地 Claude Code 或代表 Agent Ferry 承担跨目标路由。

## 后果

- V0.1 增加 daemon 的安装、自动启动、升级、日志、健康检查和本地 IPC；
- V0.1 不实现网页或云端入口，也不监听公开网络接口；
- `aferry setup` 需要安装并启动相应操作系统服务；
- 任务可以脱离扩展弹窗继续执行，扩展重新打开后可查询状态；
- Native Messaging Host 与 daemon 之间需要版本握手和不可伪造的本地连接边界；
- V0.2 仍然可以增加 ACP Managed Session，但 daemon 本身提前进入 V0.1；
- 原《开发共识》中“V0.1 不需要 Daemon”的结论由本 ADR 取代。
