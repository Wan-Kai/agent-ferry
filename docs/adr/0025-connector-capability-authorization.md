# ADR 0025：Connector 的能力授权边界

## 状态

已接受，2026-07-15。

## 背景

连接方通过认证只能证明“是谁”，不能自动获得 daemon 的全部能力。Chrome 扩展需要提交捕获内容和管理对话，但不需要安装本机软件、修改 Agent 可执行路径或接触 Hermes 凭据。若只区分已认证/未认证，扩展漏洞会直接扩大为本机管理权限。

## 决策

1. 每个已认证 `Principal` 必须携带明确的 capability 集合，业务命令在 daemon 内逐项授权；
2. V0.1 的 `ChromeNativeHostPrincipal` 固定允许：
   - `capture.submit`；
   - `target.read`；
   - `task.create`；
   - `task.read`；
   - `task.cancel`；
3. Chrome 连接方明确禁止：
   - `adapter.install` 及 adapter 更新、删除；
   - `connection.secret` 及 Hermes 凭据读写；
   - `agent.command` 及宿主可执行路径变更；
   - `daemon.admin` 及安全配置变更；
4. 上述管理操作在 V0.1 只允许本地 `aferry` CLI 调用，并继续执行各自的用户确认和系统权限检查；
5. capability 在服务端按结构化命令校验，不能依赖扩展隐藏按钮实现授权；
6. 未来 Connector 默认无权限，必须通过新的决策明确授予最小 capability 集合；
7. daemon 为拒绝的命令记录不含敏感正文的安全审计事件。

## 后果

- 扩展被利用时不能借 daemon 安装软件或读取连接密钥；
- Connector API 和命令 schema 必须能稳定映射到单一 capability；
- CLI 与 Native Host 虽然可复用 Core 用例，但使用不同 Principal 和授权策略；
- 后续 ACP 阶段若增加 `task.message` 或 `permission.respond`，必须重新做 capability 与用户手势设计，不能沿用 V0.1 权限集合。
