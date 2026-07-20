# Rust Workspace 模块地图

## 职责

维护 Rust crate 的依赖方向、跨进程协议边界和组合根约束。

## 关键成员

- `agent-ferry-protocol/`：稳定消息结构与 framing。
- `agent-ferry-transport/`：传输底座。
- `agent-ferry-core/`：共享领域与本机配置能力。
- `agent-ferry-claude/`、`agent-ferry-codex/`、`agent-ferry-opencode/`、`agent-ferry-hermes/`：目标适配器。
- `agent-ferry-daemon/`、`agent-ferry-cli/`、`agent-ferry-host/`：运行时组合根和入口。

## 依赖关系

依赖方向为基础 crate → Core → Adapter → 组合根。箭头表示上层可以依赖左侧下层；不得反向依赖，也不得在 Adapter 之间建立横向依赖。

## 不变量

- Protocol 与 Transport 不知道具体 Agent、daemon 或 UI。
- Core 不依赖具体 Adapter。
- Adapter 不直接调用其他 Adapter。
- 只有组合根选择并连接具体 Adapter。
- 凭据只通过窄接口使用，不能作为普通字符串穿过通用领域层。

## 变更影响

- 修改 Protocol 时检查 Host、daemon、CLI、Extension 和兼容性测试。
- 修改 Core 路径或配置时检查所有 Adapter、安装流程与权限。
- 新增 Adapter 时同步更新 daemon/CLI 组合、目标发现、能力展示和架构检查。

## 验证

```bash
./scripts/check-architecture
cargo test --workspace
```

## 关联文档

- [PROTOCOL] `docs/architecture/dependency-rules.md`
- `docs/architecture/overview.md`
