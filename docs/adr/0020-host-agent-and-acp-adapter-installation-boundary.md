# ADR 0020：宿主 Agent 与 ACP Adapter 的安装边界

## 状态

已接受，2026-07-15。

本决策适用于后续 ACP 阶段。根据 ADR 0026，V0.1 直接调用 Claude Code Print Mode，不安装 Claude ACP adapter。

## 背景

本地目标通常同时涉及用户实际使用的宿主 Agent 和 Ferry 所需的 ACP 连接适配层。以 Claude 为例，Claude Code 属于用户自行选择、安装、升级和认证的产品；Claude Code ACP adapter 只是 Ferry 与它建立结构化会话的连接组件。

如果 Ferry 将两者合并为一个不透明的“Claude 安装”，用户无法判断将要安装什么，也会让 Ferry 越过本机软件管理边界。

## 决策

1. 浏览器和目标列表中只展示一个用户目标 `Claude Code`，ACP 是该目标的连接方式，不是第二个 Agent；
2. 诊断信息必须分别展示 `Claude Code` 宿主和 `Claude Code ACP adapter` 的状态；
3. Claude Code 未安装时，Ferry 只显示缺失状态、官方安装指引和重新检查命令，不下载、不安装 Claude Code；
4. 只有检测到兼容的 Claude Code 后，Ferry 才提示用户安装缺失的 ACP adapter；
5. ACP adapter 只能在用户显式确认后安装，使用独立的 `adapter` 命令空间且必须明确包含 adapter 身份，例如 `aferry adapter install claude-code-acp`；
6. adapter 安装完成后执行 prerequisite、版本、ACP initialize 和 capability probe，全部通过后才启用 Claude Code 目标；
7. Claude Code 的账号、认证、升级和卸载继续由 Claude Code 自己管理，Ferry 不接管；认证检测的具体边界见 ADR 0021；
8. 这一边界适用于其他集成：Ferry 可以管理自己所需的 adapter/runtime，但默认不安装第三方宿主 Agent。

## 状态示例

```text
Claude Code                         missing prerequisite
├─ Claude Code                     not installed
└─ Claude Code ACP adapter         not evaluated

Next action
  Install Claude Code from its official distribution, then run:
  aferry agent doctor claude-code
```

```text
Claude Code                         needs adapter
├─ Claude Code                     detected
└─ Claude Code ACP adapter         not installed

Next action
  aferry adapter install claude-code-acp
```

## 后果

- Core 和 Pack 安装器不会成为第三方 Agent 的软件分发器；
- CLI 必须区分 prerequisite 缺失与 adapter 缺失，不能笼统显示 `not installed`；
- Catalog 中的 Claude 条目描述的是 Ferry adapter，而不是 Claude Code 产品；
- 文档和确认界面必须准确列出将安装的 Ferry 管理组件。
