# ADR 0013：优先接入云端 Hermes

## 状态

已接受，2026-07-15。

## 背景

用户现有的 Hermes 运行在自己的服务器上，并已连接 IM。浏览器交接的核心目标之一，是把捕获内容提交给这一现有 Hermes profile，使其可以使用服务器本地文件能力自行沉淀内容，并在后续 IM 对话中召回。

本地 Hermes 与云端 Hermes 虽然共享 Agent 名称，但部署位置、连接生命周期、认证方式和用户价值不同。若同时推进，会分散首版纵向闭环的实现与验证。

## 决策

Hermes 的接入顺序如下：

1. V0.1 优先支持用户服务器上已经运行的云端 Hermes；
2. 浏览器捕获内容直接提交给现有 Hermes profile，由 Hermes 自行决定是否以及如何持久化；
3. 本地 Hermes 不进入 V0.1，待云端链路稳定后再接入；
4. 本地 Claude 仍是独立的 V0.1 目标，本 ADR 只调整 Hermes 内部的实现优先级。

## 后果

- V0.1 的 Hermes 验收围绕云端 Gateway/API、认证、连接恢复和 IM 召回链路展开；
- 不为本地 Hermes 提前实现进程检测、启动或本地 ACP 生命周期管理；
- 领域模型必须将本地 Agent 目标与远程 Hermes Connection 明确区分；
- 本地 Hermes 可以复用未来成熟的 Managed Session 抽象，但不能阻塞首版。
