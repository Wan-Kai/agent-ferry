# ADR 0018：轻量核心包与按需 Agent Pack

## 状态

已接受，2026-07-15。

## 背景

不同 Agent 可能需要 Node runtime、adapter、平台二进制或自身 Python 环境。若把所有集成一起放入 Agent Ferry 安装包，即使用户只连接云端 Hermes，也要下载和安装完全无关的本地 Agent 组件。

用户明确要求将下载和安装尽量轻量作为持续的方案选型目标，而不是只在首版进行一次性的体积优化。

## 决策

Agent Ferry 采用轻量 Core + 按需 Agent Pack：

1. Core 只包含 daemon、Native Messaging Host、CLI、系统注册和 SQLite/Keychain 集成；ACP Client 在后续真正启用 ACP 时再按体积与复用范围决定是否进入 Core；
2. Core 不携带 Node、npm、Python、Claude、Codex、Pi 或其他 Agent runtime；
3. 已原生提供 ACP 的用户安装优先直接复用，例如 `opencode acp` 和 `hermes acp`；
4. 必须使用 adapter 的目标以独立 Agent Pack 提供，只有用户启用该目标时才下载；V0.1 的 Claude Print Mode 不需要 adapter；
5. 多个 Pack 可以共享一个受 Ferry 管理的私有 runtime，不能重复携带相同大依赖；
6. `aferry setup` 在下载前展示组件、版本、来源、体积和预计磁盘占用，并要求用户确认；
7. 所有 Pack 固定版本、校验完整性并支持升级与回滚，不允许在运行时隐式执行 `npx latest`；
8. 后续方案在功能相当时，优先选择更小的初始下载、更少的常驻进程和更低的磁盘占用。

轻量不以牺牲可复现性和供应链安全为代价。为了减少体积而依赖未固定的远程代码，不视为轻量方案。

## 后果

- 只连接云端 Hermes 的用户无需下载任何本地 Agent runtime；
- Core 与 Agent Pack 分别发布、签名和升级；
- 安装器和 doctor 需要理解 Pack 状态、版本及共享 runtime；
- 首次启用某个本地 Agent 时可能发生额外下载，但用户能提前看到并控制；
- 发布流程需要记录 Core 下载体积、安装后体积和各 Pack 增量体积，防止依赖膨胀；
- 新增 Agent 时默认不能扩大 Core，除非其代码对所有用户都是必需能力。
