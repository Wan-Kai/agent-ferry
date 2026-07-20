# Agent Ferry 开发契约

## 开始修改前

- 先运行 `git status --short`，确认并保留工作区已有变更。
- 运行 `./scripts/context <全部目标路径>`，读取根到目标目录之间的上下文链。
- 扩大修改范围时，使用新增目标路径重新运行上下文解析。

## 架构边界

- `agent-ferry-protocol` 和 `agent-ferry-transport` 是基础设施底座，不依赖上层 crate。
- `agent-ferry-core` 只依赖基础 crate，不依赖具体 Agent、daemon、CLI 或 Native Host。
- Claude、Codex、OpenCode、Hermes 适配器可以依赖 Core 和基础 crate，但不能相互依赖。
- daemon、CLI 和 Native Host 是组合根；领域与适配器不得反向依赖组合根。
- 外部系统能力通过窄 Port、显式配置和 capability 暴露，不能让凭据或协议细节下沉到通用领域代码。

## 测试与验收

- 默认测试必须离线、确定且可重复。网络、宿主 Agent、Keychain、时间和随机值只在确有需要的边界使用可替换实现。
- 优先在最高可行边界验证可观察行为，包括进程参数、stdin、IPC frame、HTTP/SSE、文件权限和 UI 状态。
- `./scripts/verify` 是本地完整工程门禁；CI 必须调用同一入口。
- 本地门禁通过不代表真实 Chrome、macOS Keychain、宿主 Agent 或远程 Hermes 已验收。真实环境验证按 `docs/runbooks/real-environment-acceptance.md` 单独记录。

## 文档与证据

- Current 文档描述当前代码；ADR 解释长期决策；Draft 不得冒充已实现能力；Historical 只提供背景。
- 代码、测试与 Current 文档冲突时，先判断是实现回归还是文档过时，并在同一变更中修正。
- Accepted ADR 被改变时必须新增替代 ADR，并把旧 ADR 标记为 Superseded。
- 发布证据必须绑定干净的完整 commit，并记录 Rust、Node 版本和产物 SHA256。普通开发验证可以在脏工作区运行，但不能作为 RC 证据。

## 代码注释

- 所有代码注释使用中文，专有名词和代码标识符可以保留英文。
- 注释解释业务背景、设计理由、边界、兼容性、安全、并发和修改风险，不复述代码行为。
- L3 文件契约只用于组合根、协议状态机、凭据、进程生命周期等高风险文件，避免给简单代码制造注释噪声。

## 常用命令

```bash
./scripts/context <目标路径...>
./scripts/verify
cargo test -p <crate-name>
npm --prefix extension test
```
