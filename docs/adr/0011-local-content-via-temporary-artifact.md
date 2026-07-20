# ADR 0011：本地 Claude 通过临时 Artifact 读取完整内容

## 状态

已接受，2026-07-15。

## 背景

论文和长文可能超过 CLI stdin 或单条消息适合承载的长度。将完整正文直接拼入 Prompt 也会降低用户检查任务说明的可读性，并使重试和故障诊断变得困难。

本地 Claude 已在用户选择的 Workspace 中运行，但捕获内容本身不一定属于该项目，也不应默认污染项目目录或 Git 工作区。

## 决策

本地 Claude 交接采用 daemon 管理的临时 Markdown Artifact：

1. `agentferryd` 在操作系统提供的临时目录下创建按 Handoff 隔离的目录和内容文件，不硬编码 `/temp` 或 `/tmp`；
2. Print Task 的首条 Prompt 包含用户可见的有效 Prompt、来源信息、Artifact 绝对路径，以及明确读取该文件的要求；
3. Claude 以目标 Workspace 为工作目录启动，但通过绝对路径读取临时 Artifact；
4. V0.1 的 `unrestricted_host` 不限制 Workspace 外读取；后续 sandbox 模式必须自行解决如何安全挂载或复制 Artifact；
5. daemon 负责限制文件权限、清理过期内容，并在任务状态中区分 Artifact 创建、会话启动和 Agent 读取结果；
6. Artifact 在任务进入结束状态后默认保留 24 小时。运行中的任务不进入过期清理。

该临时文件只用于本地 Agent 的内容传递。远程 Hermes 仍通过其程序化接口接收内容，并自行决定是否以及如何持久化。

## 后果

- 浏览器扩展无需把大段正文直接塞入 CLI Prompt；
- 项目工作区不会因为普通交接产生未跟踪文件；
- daemon 必须管理临时文件的权限、容量、过期清理和崩溃恢复；
- 系统临时目录可能被操作系统提前清理，因此 daemon 在会话需要再次读取时必须能识别内容已丢失并明确报错；
- daemon 应在启动时和运行期间定时清理超过保留期的 Artifact，不能依赖正常退出；
- 24 小时是默认值，未来可以允许用户调整，但不能配置为无期限保存而仍称为临时内容。
