# ADR 0006：Handoff 使用目标专属配置

## 状态

已接受，2026-07-14。

## 背景

本地 Claude Code 必须在用户配置的固定工作区中启动，因此需要 `workspace_id`。远程 Hermes 已经作为长期服务运行，并自行决定服务器上的文件位置，因此只需要连接和 profile，不需要 Ferry 管理的工作区路径。

如果继续把 `workspace_id` 设计为所有 Handoff 的必填字段，扩展会向 Hermes 用户展示无意义的目录选项，协议也会产生伪造或空值状态。

## 决策

Handoff 使用带类型判别的目标配置，而不是平铺的 `workspace_id` 和 `agent_id`：

```text
HandoffTarget
├── LocalCli
│   ├── agent_id
│   └── workspace_id
└── RemoteHermes
    └── connection_id
```

扩展根据目标类型展示字段：本地 CLI Agent 必须选择工作区；远程 Hermes 只选择已配置的 Hermes 实例。目标配置解析完成后，核心流程才能进入内容持久化或提交阶段。

## 备选方案

### 所有目标都要求 Workspace

不采用。它把本地 CLI 的启动约束错误地施加给长期远程服务。

### 使用多个可空字段

不采用。`workspace_id`、`connection_id` 和其他字段任意组合会产生无效状态，增加协议校验负担。

## 后果

- 当前 Rust `Handoff` 模型需要在实现阶段调整；
- 扩展表单根据目标类型切换，不为 Hermes 展示 Workspace；
- 本地 artifact 写入与远程 Hermes 内容提交是两条不同执行路径，但共享捕获内容、有效 Prompt 和结果阶段模型；
- 未来新增其他服务型 Agent 时，可以增加新的目标变体而不伪装成本地工作区。
