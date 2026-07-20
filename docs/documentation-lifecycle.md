# 文档生命周期与事实优先级

> 状态：Current
> 事实来源：仓库代码、测试、Cargo 配置和 Accepted ADR
> 范围：根文档、上下文地图、架构、Runbook、ADR、设计和研究文档

## 事实优先级

发生冲突时先调查，不直接假定代码或文档天然正确：

1. 运行代码、测试、构建和真实外部契约；
2. `AGENTS.md`、`CONTEXT.md` 和 Current 架构文档；
3. Accepted ADR；
4. Current Runbook；
5. Draft 设计与 PRD；
6. Historical 设计、研究和已完成工作记录。

若代码与 Current 文档冲突，应通过测试、调用方和 ADR 判断是实现回归还是文档过时，并在同一变更中修正。Accepted ADR 的结论发生改变时新增替代 ADR，旧文档标记为 Superseded 并链接替代者。

## 状态

- `Current`：描述当前实现或当前操作方式，行为变化时同步更新。
- `Draft`：讨论中或尚未完整实现，不能声称运行时已经具备。
- `Historical`：保留当时范围与背景，不能作为当前实现证据。
- `Superseded`：已被明确替代，必须链接到替代文档。
- `Accepted`：ADR 已接受并生效；被替代后转为 Superseded。
- `Proposed`：ADR 尚未接受。

非 ADR 文档在标题后使用：

```markdown
> 状态：Current | Draft | Historical | Superseded
> 事实来源：代码路径、ADR 或上级设计
> 范围：覆盖和不覆盖的内容
```

ADR 使用 `## 状态`，并明确写出 Accepted、Proposed 或被哪个 ADR Superseded。

## 更新触发

| 文档 | 默认状态 | 更新触发 |
|---|---|---|
| `AGENTS.md` / `CONTEXT.md` | Current | 职责、依赖、不变量变化 |
| `docs/architecture/` | Current | 数据流、模块、公共接口变化 |
| `docs/adr/` | Accepted / Proposed | 长期决策改变时新增替代 ADR |
| `docs/runbooks/` | Current | 命令、环境、配置或验收方式变化 |
| `docs/design/` / `docs/prd/` | Draft / Historical | 设计推进或被实现替代 |
| `docs/research/` | Historical | 通常保留原始时间点结论 |

所有新增文档加入 `docs/README.md` 或下级索引，并通过 `./scripts/check-docs` 与 `./scripts/check-links`。
