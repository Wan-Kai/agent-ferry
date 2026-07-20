# 本地开发与验证

> 状态：Current
> 事实来源：Cargo workspace、Extension package scripts 和 CI
> 范围：开发环境、局部测试与完整离线门禁

## 工具链

- Rust 最低版本由根 `Cargo.toml` 的 `rust-version` 声明。
- Node.js 使用 CI 配置的主版本，目前为 Node 22。
- 正式用户使用预编译产物；这些工具链只属于仓库开发环境。

## 修改前

```bash
git status --short
./scripts/context <全部目标路径...>
```

扩大范围时重新运行 `context`，让新增目录的 `AGENTS.md` 进入当前上下文。

## 局部验证

```bash
cargo test -p agent-ferry-daemon
cargo test -p agent-ferry-protocol
npm --prefix extension test
```

测试默认不得依赖公网、真实 Token、真实 Hermes 或用户已有 Agent 状态。宿主进程通过 fake executable，远程服务通过本机 fake HTTP/SSE，文件和配置使用隔离临时目录。

首次 clone 后可以启用仓库维护的快速 pre-commit：

```bash
./scripts/install-git-hooks
```

Hook 只检查格式、架构和文档契约，不替代完整门禁。

## 完整门禁

```bash
./scripts/verify
```

完整门禁依次检查工具链、Rust 格式、Clippy、Rust workspace 测试、Extension 测试、TypeScript、生产构建、架构边界、分形文档契约和本地链接。CI 调用同一个入口，避免本地与 CI 漂移。

生成可审计的本地证据：

```bash
./scripts/verify --evidence target/verification-evidence.json
```

RC 或发布证据必须使用干净工作区，并绑定实际产物：

```bash
./scripts/verify --require-clean \
  --evidence target/release-evidence.json \
  --artifact extension/dist/agent-ferry-extension-0.1.0-chrome.zip \
  --artifact target/release/aferry
```

本门禁只证明离线工程闭环。真实 Chrome、Keychain、宿主 Agent 和 Hermes 按真实环境 Runbook 单独验收。
