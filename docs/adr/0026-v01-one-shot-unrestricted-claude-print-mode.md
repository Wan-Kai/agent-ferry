# ADR 0026：V0.1 使用 Claude Print Mode 执行一次性不受限任务

## 状态

已接受，2026-07-15。

取代 ADR 0009、ADR 0010 和 ADR 0017 在 V0.1 中的决定。ACP 托管会话延期到后续版本。

## 背景

完整 ACP 会话需要消息续写、权限审批、事件恢复、adapter 安装和会话 UI，显著扩大首版范围。V0.1 的核心价值可以由更窄的交接闭环验证：浏览器提交一个任务，Claude Code 自主执行，结束后返回最终输出，用户中途不参与对话。

用户明确要求首阶段的本地 Claude 与其在本机直接运行时拥有相同权限，Ferry 不增加工具白名单；未来再单独设计沙箱模式。

## 决策

1. V0.1 直接调用用户已经安装并认证的 Claude Code CLI，不安装 `claude-code-acp`；
2. daemon 在用户选择的 Workspace 中以结构化 argv 启动一次性非交互进程：

   ```text
   claude -p --permission-mode bypassPermissions --output-format stream-json --verbose
   ```

   daemon 将用户可见的 Prompt 写入子进程 stdin 后关闭输入，不把 Prompt 放进进程参数或 shell 命令。实现可使用与之等价的 `--dangerously-skip-permissions`，但配置和 UI 统一称为 `unrestricted_host`；
3. Ferry 不传入 `--tools`、`--allowedTools` 或额外权限规则，不限制 Claude 的读写、Shell 或网络工具；
4. 实际权限仍受当前操作系统用户权限、Claude Code managed settings 和 Claude 自身不可绕过的保护约束，Ferry 不宣称获得 root 或绝对无限权限；
5. Workspace 是启动目录和项目上下文，不是安全沙箱。Claude 可能访问当前用户有权访问的 Workspace 外路径；
6. 捕获正文继续写入 daemon 管理的临时 Markdown artifact，Prompt 显示其绝对路径并要求 Claude 读取；
7. daemon 解析 `stream-json` 以记录启动、输出、重试、错误、费用和最终结果，但浏览器首版只展示运行状态与最终输出；每次交接使用新的 task/session UUID，不使用 `--continue` 或 `--resume`；
8. 用户不能在任务运行中追加消息、回答澄清问题或处理权限请求；唯一的中途操作是取消整个任务；取消时 daemon 终止 Claude 进程组，在短暂优雅退出窗口后强制清理仍存活的子进程；
9. Claude 进程退出时任务进入 `succeeded`、`failed` 或 `cancelled`，不保留可继续对话的 Ferry 会话；取消前收到的片段可用于诊断，但不能标记为最终答案；
10. 本地 Claude 任务默认最长运行 60 分钟；超过时限后终止进程组并进入独立的 `timed_out` 状态，不能混同为用户取消或 Agent 失败；
11. V0.1 浏览器不提供超时设置；后续可以通过 CLI 修改默认值，并在任务创建时固化实际 deadline，配置变化不追溯影响运行中任务；
12. 不默认使用 `--bare`，以继续沿用用户现有 Claude Code 登录、Keychain、CLAUDE.md 和项目配置；
13. 不传 `--no-session-persistence`，保留 Claude Code 自己的默认原生会话记录；Ferry 不读取、展示、索引或依赖该记录；
14. V0.1 不实现 sandbox。后续将 `sandboxed` 作为独立执行模式设计，不在 `unrestricted_host` 上逐项叠加临时限制。

## 安全边界

网页内容属于不可信输入，而 V0.1 Print Task 可以使用当前用户权限执行操作。V0.1 不增加单独的 execution mode 配置、启用确认或逐任务确认；本地 Claude 目标固定使用该运行方式，目标诊断和产品文档必须准确说明。Ferry 对正文与控制 Prompt 做结构分隔，但这不是安全边界，不能声称可以阻止所有 prompt injection。

## 后果

- V0.1 不需要 ACP Client、Claude ACP adapter、浏览器权限审批或多轮会话 UI；
- Claude Code 缺失时仍只提供官方安装指引，Ferry 不安装宿主产品；
- 任务接口可以保持 `starting → running → succeeded/failed/cancelled/timed_out`，不因 Workspace 相同进入产品队列；
- 本机权限风险高于只读或沙箱模式；V0.1 接受这一临时边界以优先打通闭环，后续 ACP 和 sandbox 设计必须重新处理权限；
- ACP、Agent Pack 与 adapter CLI 设计保留为后续扩展，不进入首版交付范围。
