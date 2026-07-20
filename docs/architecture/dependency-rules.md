# Rust Crate 依赖规则

> 状态：Current
> 事实来源：Cargo metadata 与 `scripts/check-architecture-boundaries.mjs`
> 范围：Workspace 内部 crate 的直接依赖方向

## 分层

```text
Foundation: agent-ferry-protocol, agent-ferry-transport
Domain:     agent-ferry-core
Adapters:   agent-ferry-claude, agent-ferry-codex,
            agent-ferry-opencode, agent-ferry-hermes
Roots:      agent-ferry-daemon, agent-ferry-cli, agent-ferry-host
```

规则：

- Foundation 不依赖任何其他 workspace crate。
- Domain 只依赖 Foundation。
- Adapter 只依赖 Foundation 与 Domain，不依赖其他 Adapter 或组合根。
- 组合根可以依赖下层；组合根之间不得建立生产依赖。
- CLI 和 Native Host 的集成测试可以通过 dev-dependency 启动 daemon。

Cargo 自身只能阻止循环依赖，不能阻止无环但方向错误的依赖。`./scripts/check-architecture` 对上述语义边界提供额外门禁。当前没有历史违规，因此不设置债务白名单；未来如确需冻结历史债务，必须逐条说明原因且只能减少。
