# Agent Ferry 当前架构

> 状态：Current
> 事实来源：Cargo workspace、Extension 入口、Protocol 与 daemon 测试
> 范围：当前代码已经存在的组件和数据流，不描述未来 ACP 或云端控制面

## 系统上下文

```text
Chrome 当前页面
  -> WXT Extension
  -> Chrome Native Messaging
  -> agentferry-host
  -> 当前用户私有 Unix Socket
  -> agentferryd
       -> 本机 Claude Code
       -> 本机 Codex CLI / Codex App
       -> 本机 OpenCode
       -> Remote Hermes Direct / SSH Tunnel
```

Extension 负责捕获、Prompt、目标选择和 UI。Native Host 只处理 framing 与 IPC 转发。daemon 是运行时组合根，负责 Connector 认证授权、目标发现、任务执行、事件归一化和有界历史。具体 Agent crate 隔离宿主协议、进程参数和兼容性检测。

## 内容与控制边界

内容面承载页面来源、用户可见 Prompt 和正文。大正文经有界分块发送到 daemon；页面文本不得决定可执行文件、argv、Workspace、连接或 capability。

控制面承载状态、目标发现、任务命令与事件。Protocol 是 Extension、Host、daemon 和 CLI 之间的稳定契约；具体 Agent 的原始事件在 Adapter/daemon 边界归一化。

## 持久数据

- Workspace、Agent 绑定、Hermes Connection 元数据和有界历史默认保存在权限为 `0700` 的 `~/.agent-ferry`。测试与开发可以通过 `AGENT_FERRY_HOME` 使用隔离目录。
- Hermes Secret 通过 `CredentialStore` 使用，正式构建存入 macOS Keychain。
- daemon 以仅当前用户可访问的 JSON 文件保存有界任务历史，包括 Prompt、来源摘要、目标快照、状态、输出和非敏感错误；不保存捕获正文或凭据。
- 宿主 Agent 自身的会话、文件和认证仍由宿主拥有。

早期开发版本的 `~/Library/Application Support/Agent Ferry` 不会在进程启动时静默搬移。正式安装流程在停止旧 daemon 后调用 `aferry data migrate`；双目录并存或符号链接会被拒绝，避免覆盖或越界移动用户数据。

## 组合根

- daemon 组合实时任务运行路径。
- CLI 组合安装、诊断与管理路径，并通过 `aferry service` 管理当前用户的 macOS LaunchAgent；服务 plist 与日志遵循平台目录，卸载服务不删除用户数据。
- Native Host 是受限入口，可在测试中以 daemon 作为开发依赖建立进程级契约。

依赖方向的机械规则见 [依赖规则](./dependency-rules.md)。
