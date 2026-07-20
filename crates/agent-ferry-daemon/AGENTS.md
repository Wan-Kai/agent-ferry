# agentferryd 模块地图

## 职责

作为本机交接中枢认证 Connector、校验 capability、组装目标适配器、管理任务生命周期，并持久化有界任务历史。

## 关键成员

- `src/lib.rs`：Unix Socket、命令分发、授权、目标发现和任务组合。
- `src/history.rs`：有界历史记录、崩溃后中断标记和原子落盘。
- `tests/`：IPC、分块传输、Workspace 和各目标端到端契约。

## 依赖关系

daemon 是组合根，可以依赖 Core、Protocol 和各 Adapter。下层 crate 不能依赖 daemon。Chrome Native Host 只通过 Protocol 与私有 Socket 调用 daemon。

## 不变量

- Chrome Principal 只能调用显式允许的业务命令，管理命令只能由 CLI Principal 使用。
- 页面正文、Prompt、Token 和完整 Agent 输出不得写入普通日志。
- 分块正文必须验证 task、顺序、总大小、数量和 SHA256，再进入目标适配器。
- 不同任务相互隔离；取消、失败和历史更新不能串扰其他 task id。
- 历史文件权限为当前用户私有，输出和记录数量保持有界，运行中记录不可删除。

## 变更影响

- 新增命令时同步修改 Protocol、capability 授权、Host/CLI 调用方和 IPC 测试。
- 新增目标时同步修改发现、诊断、事件归一化、历史快照和 Extension 展示。
- 修改历史字段或保留策略时同步修改 Protocol、Extension、迁移兼容与隐私文档。

## 验证

```bash
cargo test -p agent-ferry-daemon
./scripts/check-architecture
```

## 关联文档

- [PROTOCOL] `docs/architecture/overview.md`
- `docs/adr/0024-authenticated-daemon-connectors.md`
- `docs/adr/0025-connector-capability-authorization.md`
- `docs/adr/0030-bounded-local-task-history.md`
