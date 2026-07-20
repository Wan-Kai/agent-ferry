# Agent Ferry 术语表

> 状态：Current
> 事实来源：当前 Protocol、架构文档和 Accepted ADR
> 范围：代码、测试和产品文档共用的领域词汇

本文记录产品讨论中已经形成明确含义的领域术语。术语含义变化时，应同步更新本文和相关 ADR。

## 捕获内容（Captured Content）

用户在浏览器中阅读并希望交给 Agent 的实际内容。V0.1 的主要来源是论文、Twitter/X 帖子和技术文档，至少包含页面标题、URL、正文或选区。URL 是来源元数据，不能代替正文。

## 通用整页捕获（Generic Full-page Capture）

普通站点使用的默认捕获策略：在有限时间和内容量内尽可能触发懒加载，再提取浏览器页面中可读取的内容。它不等同于保证抓取无限列表中的全部数据。

## 内容提取器（Content Extractor）

将浏览器页面或原始文档转换为结构化捕获内容的组件。普通页面使用 Defuddle 通用提取；Twitter/X、YouTube 和 PDF 等来源可以使用专用提取器。每次捕获应记录实际使用的提取器和完整性状态。

## 交接（Handoff）

Agent Ferry 将捕获内容与用户目标送入指定目标，启动或连接 Agent，并自动提交首条消息的完整过程。内容物化、Agent 启动和首条消息提交是可独立成功或失败的阶段。

## 工作区（Workspace）

Agent 执行任务时使用的固定目录。工作区可以位于本机，也可以位于用户自己的远程服务器。

## 本地 Agent（Local Agent）

在用户本机运行的 Agent。V0.1 的首要本地 Agent 是 Claude Code，由 `agentferryd` 在选定 Workspace 中启动一次性 Print Task。

## Print Task

通过 Agent 原生 CLI 的非交互输出模式执行的一次性任务。V0.1 使用 `claude -p`：每次交接创建独立 Claude 对话，用户只能提交首条任务、查看状态与最终输出或取消整个进程，不能续写消息、处理中途审批或恢复为 Ferry 会话。同一 Workspace 的多个 Print Task 可以并行。

## 实时结果流（Live Result Stream）

V0.1 将 Claude Print Mode 输出只转发给当前连接的浏览器界面，不保存回复正文、最终答案或可重放历史。界面断开期间的内容可能永久丢失；会话持久化留到 ACP 阶段。

Claude Code 自己按原生默认行为保存的会话记录不属于 Ferry 的结果历史。Ferry 不禁用它，也不读取、展示或依赖它。

## unrestricted_host

V0.1 本地 Claude 的执行模式。Ferry 不添加工具、Shell、网络或文件路径限制，Claude 以当前操作系统用户权限执行；Workspace 只是启动目录而不是沙箱。Claude Code managed policy 与其自身不可绕过的保护仍可能生效。

## timed_out

一次性任务超过创建时固化的最长运行时间后，由 daemon 终止进程组形成的结束状态。它与用户主动产生的 `cancelled` 和 Agent 自身错误产生的 `failed` 分开记录。V0.1 本地 Claude 默认时限为 60 分钟。

## 托管会话（Managed Session）

由 `agentferryd` 通过 ACP 或 Agent 原生 RPC 持有的结构化会话。daemon 负责创建 Session、提交和继续发送消息、转发流式内容与工具事件、处理权限请求和取消，并在浏览器扩展关闭后继续维持任务；它不等同于托管或实现 Agent 自身的推理循环。

## 待审批（Awaiting Approval）

托管会话收到敏感操作权限请求后的暂停状态。daemon 在扩展弹窗关闭后继续保存请求，通过 Chrome 通知和扩展角标提醒用户；只有用户明确批准后才能继续，拒绝或超时都按不授权处理。

## 远程 Agent（Remote Agent）

在用户服务器上运行的 Agent。V0.1 的首要远程 Agent 是 Hermes，其内容传输和启动需要经过安全的远程连接。

## 云端 Hermes（Remote Hermes）

用户部署在自己服务器上、并已连接 IM 的 Hermes 实例。它是 V0.1 优先支持的 Hermes 形态；本地 Hermes 在云端链路稳定后再接入。这里的“云端”描述部署位置，不表示由 Agent Ferry 提供托管服务。

## Hermes 持久导入（Hermes Ingestion）

Agent Ferry 将有效 Prompt、来源信息和完整 Markdown 直接放入 Runs API `input`，提交给现有 Hermes profile；Hermes 使用自己的服务器本地文件能力决定是否保存、保存到哪里以及如何建立索引。Agent Ferry 不直接访问远程文件系统，也不在首版实现分块上传。

## 可召回（Recallable）

用户以后在 IM 中围绕文档提问时，Hermes 能够通过自己维护的文档、索引、会话记录或记忆找到相关内容。

## Hermes 连接（Hermes Connection）

Agent Ferry 用于连接现有远程 Hermes profile 的配置。它将 API Endpoint、认证引用和网络 Transport 分开：优先 Direct URL，SSH Tunnel 作为回退。它声明该实例的实际能力，但不包含由 Ferry 管理的远程工作区路径。

## Handoff 目标（Handoff Target）

一次交接的唯一执行目的地。它是带类型判别的配置：本地 Claude 目标由 Agent 和 Workspace 组成；云端 Hermes 目标由 Hermes Connection 组成。不同目标不能通过无意义的空字段互相伪装。V0.1 的 Handoff 不支持目标列表。

## 交接中枢（Handoff Hub）

运行在用户本机的 `agentferryd`。V0.1 只接受浏览器扩展的交接请求，管理任务状态、目标连接和路由，但不执行 Agent Loop。其他入口属于未来扩展。

## Native Messaging Bridge

`agentferry-host` 承担的薄桥接角色。它处理 Chrome Native Messaging framing 和扩展来源校验，再通过本地 IPC 把请求转发给 `agentferryd`；它不独立管理长期任务或远程连接。

## 首发客户端（Initial Client Platform）

首个可用版本正式支持的本地运行平台，即 macOS。它包含浏览器、Native Host、daemon 和本地 Claude Code；远程 Linux Hermes 是目标服务，不属于客户端平台范围。

## Agent Ferry Core

所有用户都需要的轻量安装包，只包含 daemon、Native Messaging Host、CLI、系统注册和 Keychain 集成。V0.1 不需要任务历史数据库，也不包含 ACP Client 或特定 Agent runtime。

## Agent Pack

用户启用特定本地 Agent 时按需下载的固定版本组件，例如 ACP adapter、平台二进制或私有 Node runtime。下载前必须展示来源、版本和体积；多个 Pack 应共享相同 runtime，不能把可选依赖塞入 Core。

## Component

由 Agent Pack 精确引用的不可变、可复用文件集合，例如共享 Node runtime 或某个版本的 ACP adapter。Component 按平台和版本寻址，安装前校验签名与 hash，相同 Component 在多个 Pack 之间只保存一份。

## LaunchSpec

Pack Manager 从声明式 manifest 解析出的安全启动描述，包含绝对可执行文件、结构化参数、Workspace、受控环境变量和协议类型。它不包含 shell 命令，也不允许 Pack 在安装时执行任意脚本。

## Agent Catalog

由 Agent Ferry 签名维护的可安装 Adapter/Agent Pack 索引。`aferry adapter list --available` 使用它展示当前支持项、固定版本和体积；Catalog 只提供元数据，不会触发后台自动安装。

## Artifact

由捕获内容生成、可供用户和 Agent 检查的文件。本地 Claude 的 V0.1 Artifact 默认位于操作系统提供的临时目录，不写入项目工作区；任务结束后默认保留 24 小时，由 daemon 清理。运行中的任务不进入过期清理。远程 Hermes 不依赖该文件路径。

## Prompt 模板（Prompt Template）

用户在浏览器扩展中配置的可复用任务说明，例如总结论文、解释方法或评估内容与当前项目的关系。用户可以在交接时选择模板，但模板不是必填项。模板解析后填入最终 Prompt 编辑框；对该文本的单次修改不会覆盖模板。

## 有效 Prompt（Effective Prompt）

一次交接最终提交给 Agent 的任务说明。它来自用户选择并解析后的模板、系统内置默认 Prompt 或用户在编辑框中的修改。扩展必须在交接前展示其完整内容，不能包含用户不可见的隐藏任务指令。
