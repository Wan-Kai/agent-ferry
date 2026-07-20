# ADR 0005：远程文档持久化由 Hermes 自主管理

## 状态

已接受，2026-07-14。

## 背景

远程 Hermes 已经运行在用户服务器上，并通过 IM Gateway 与用户持续交互。Hermes 本身能够操作服务器本地文件，也可以根据自己的规则决定文档目录、摘要和索引方式。

如果 Agent Ferry 再通过 SSH/SFTP 管理远程 inbox 或长期知识目录，会复制 Hermes 已有的文件管理能力，并将 Ferry 与用户自定义的 Hermes 目录结构耦合。

## 决策

Agent Ferry 通过 Hermes 的程序化接口把捕获内容和有效 Prompt 直接提交给正在运行的 Hermes profile。Prompt 要求 Hermes 分析内容，并根据自身规则判断是否持久化、保存到哪里以及如何建立后续召回所需的索引。

Agent Ferry 不直接访问 Hermes 服务器文件系统，不建立远程 inbox，不写 `MEMORY.md`，也不规定 Hermes 的长期知识目录。远程交接成功以 Hermes 接受并完成本次任务为准，而不是以 Ferry 观察到某个固定文件为准。

Hermes 连接配置必须能够声明该实例是否允许使用本地文件工具。扩展在交接前应展示目标 Hermes 实例；文件写入的具体路径属于 Hermes 的执行结果，不属于浏览器输入。

## 备选方案

### Agent Ferry 维护远程 inbox

不采用。该方案需要额外的 SSH 文件传输、目录所有权和清理协议，也限制 Hermes 自己演进知识管理方式。

### Agent Ferry 直接写 Hermes memory

不采用。内置 memory 容量有限，且绕过 Hermes 自己的判断与安全机制。

### 只发送 URL

不采用。Hermes 服务器不一定具备浏览器登录态，必须发送浏览器捕获到的实际内容。

## 后果

- 远程 Hermes 目标由连接端点和 profile 标识，不要求用户选择远程工作区路径；
- 大型 HTML、Markdown 和 PDF 内容需要由 Hermes 接口支持可靠传输或附件提交；
- Ferry 无法通过固定路径直接验证持久化结果，需要依赖 Hermes 的结构化任务结果；
- Hermes 实例没有文件工具或相应权限时，仍可完成分析，但不能声称已经持久保存；
- 远程连接的认证、加密与权限配置成为 V0.1 顶层设计的一部分。
