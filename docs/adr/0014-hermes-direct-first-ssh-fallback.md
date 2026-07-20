# ADR 0014：云端 Hermes 直连优先，SSH Tunnel 回退

## 状态

已接受，2026-07-15。

## 背景

社区通常把远程 Agent 配置为 Gateway URL 与应用层鉴权，再根据网络环境选择私网直连、SSH Tunnel 或公网 HTTPS。强制 SSH 会增加端口和子进程管理成本，也无法覆盖未来 Web 或移动入口；只支持公网 URL 又会迫使用户暴露具备文件和终端能力的 Agent API。

Hermes API Server 已提供 Runs API、SSE、审批、取消、会话关联和能力发现，满足浏览器交接需要，不必远程模拟 CLI。

## 决策

V0.1 使用 Hermes API Server 的 Runs API 作为云端 Hermes 的首选控制接口。Hermes Connection 将 Endpoint 与 Transport 分开：

1. Endpoint 保存 API Base URL、目标 profile/model 和 API Key 引用；
2. `direct` 是优先传输，支持 Tailscale、WireGuard、可信局域网或用户自行配置的 HTTPS Endpoint；
3. `ssh_tunnel` 是内建回退，复用用户有效的 `~/.ssh/config`，并将服务器 loopback API 转发到 daemon 分配的本地端口；
4. 两种传输都必须使用 Hermes Bearer Token，SSH 登录不能替代 Hermes API 鉴权；
5. daemon 连接成功后先请求 `/v1/capabilities`，只启用服务器实际声明支持的状态、SSE、审批和取消能力；
6. OAuth/OIDC 公网登录和 Agent Ferry 自建云中继不进入 V0.1。

API Key 的值保存在 macOS Keychain，项目配置和浏览器扩展只持有凭据引用。

## 后果

- 已有 Tailnet 或 HTTPS 的用户只需提供 API URL 和凭据；
- 只有 SSH 可达的服务器仍可保持 Hermes API 监听 loopback；
- daemon 需要为 SSH 模式实现严格 host key 校验、端口分配、断线重连和子进程回收；
- Transport 只解决网络可达性，不改变 Hermes Session、Profile 或持久化语义；
- 未来增加其他安全传输时无需修改 Handoff 领域模型。
