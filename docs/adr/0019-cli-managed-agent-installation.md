# ADR 0019：通过 aferry 命令管理 Agent 接入

## 状态

已接受，2026-07-15。

Adapter 安装命令保留为后续 ACP 阶段设计；V0.1 的 Claude Print Mode 不安装 adapter。

## 背景

Agent Pack 按需安装可以保持 Core 轻量，但用户仍需要一个清晰、可检查和可自动化的入口来了解当前支持范围、安装缺失组件、检测原生 Agent 并诊断故障。

让 Chrome 扩展直接下载和安装 runtime 会混淆浏览器权限与本机软件管理边界，也不利于展示签名、版本、体积和诊断日志。

## 决策

Agent 接入统一通过 `aferry` CLI 管理：

1. `aferry setup` 在首次配置时展示官方支持的 Agent、当前安装状态和下一步命令；它是只读的检查与引导命令，不下载或安装 Agent Pack；
2. `aferry agent list` 展示目标的已启用、已检测、缺少前置条件、缺少 adapter、需配置和不兼容状态；
3. `aferry adapter list/install/update/remove` 只管理 Ferry 自身的固定版本 ACP adapter、Pack 和共享 Component，不安装第三方宿主 Agent；
4. `aferry agent enable/disable <id>` 启用或停用已就绪的 Agent 目标；
5. `aferry agent doctor [id]` 分别检查宿主、adapter、命令解析、版本、ACP initialize 和 capability；
7. 云端 Hermes 不是本地 Pack，通过 `aferry connection add hermes` 配置 Endpoint、Transport 和 Keychain 凭据；
8. 安装成功后立即自动运行 doctor，只有 probe 通过才切换为 active；
9. Chrome 扩展不安装软件，只展示 daemon 已确认可用的目标和诊断入口。

CLI 在下载前显示组件来源、版本、增量下载量和磁盘占用，并要求确认。`--yes` 只用于用户显式运行的非交互自动化；`--json` 提供稳定的机器可读输出。

下载只能由用户显式执行 `aferry adapter install <adapter-id>` 触发。`setup`、`list`、`doctor`、daemon 启动和浏览器任务执行均不得隐式安装或升级组件。

对于依赖第三方宿主 Agent 的目标，CLI 必须分别显示宿主和 adapter 状态。宿主缺失时只提供官方指引；宿主存在但 adapter 缺失时，才提供明确的 adapter 安装命令。详细边界见 ADR 0020。

## 后果

- Core 首次运行即可告诉用户“现在能用什么、缺什么、下一条命令是什么”；
- 浏览器扩展不需要文件下载、解压或系统安装权限；
- 安装逻辑只有 Pack Manager 一份，CLI 和未来 GUI 都调用相同用例；
- CLI 输出需要稳定状态枚举和可操作的修复建议；
- daemon 运行时不得自行安装 Pack，只能报告缺失或不兼容；
- 未来 GUI 安装仍可复用同一 Core API，但不能绕过确认、签名和 probe。
