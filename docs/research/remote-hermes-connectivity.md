# 云端 Hermes 连接方式调研

> 状态：Historical
> 事实来源：2026-07-15 调研
> 范围：远程连接方案背景；当前行为以 Hermes Adapter、测试和 Current 架构为准

调研日期：2026-07-15。

## 结论摘要

开源社区的共同做法不是强制所有用户使用同一种隧道，而是把 Agent 客户端配置为“远程 Gateway URL + 应用层鉴权”，再按网络环境选择传输路径：

1. 已有 Tailscale、WireGuard 或可信局域网时，直接连接私网地址；
2. 只有 SSH 可达时，通过本地端口转发连接服务器 loopback Gateway；
3. 必须从公网访问时，使用 HTTPS/WSS、反向代理和 OAuth/OIDC 等强身份认证；
4. 厂商托管的 Dev Tunnel 可以降低配置成本，但会引入账户和中继服务依赖。

因此 Agent Ferry 的领域模型应把 Hermes API Endpoint 与网络传输方式分开，不能把 `ssh_host` 写死为所有 Hermes Connection 的必填字段。

## Hermes 官方能力

Hermes 提供三类程序化接口：

- ACP：JSON-RPC over stdio，主要用于在同一台机器上由 IDE 启动 Agent；
- TUI Gateway：stdio 或 WebSocket，提供完整的 session、approval、clarify 和流式事件控制；
- API Server：HTTP + SSE，面向语言无关客户端和 OpenAI-compatible frontend。

对于 Agent Ferry，API Server 的 Runs API 已直接提供创建任务、查询状态、订阅 SSE、审批和停止任务；`/v1/capabilities` 可以进行运行时能力发现。API Server 默认绑定 `127.0.0.1:8642`，所有部署都要求 `API_SERVER_KEY` Bearer Token，并默认不开启浏览器 CORS。Ferry 由本地 daemon 连接，不需要开启 CORS。

Hermes Desktop 的远程方案使用可配置 Remote URL。官方建议可信网络使用 Tailscale；公网后端使用 OAuth/OIDC，不建议把仅用户名密码保护的 Agent 后端直接暴露到公网。

## 同类社区方案

### 私网直连

OpenClaw 将 LAN/Tailnet WebSocket 视为最简单的远程路径，推荐 Gateway 保持在可信局域网或 Tailnet 中。Hermes Desktop 同样把 Tailscale 作为可信网络远程连接的推荐方式。

优点：

- daemon 直接使用稳定 URL，连接和重连简单；
- 不需要为每个客户端维持 SSH 子进程；
- 适合长期运行、SSE 和 WebSocket。

代价：

- 用户需要预先安装并配置 Tailscale/WireGuard；
- Ferry 不应接管用户的 VPN 账户和网络管理。

### SSH Tunnel

OpenClaw 把 SSH Tunnel 定义为普适回退路径，并允许 macOS 客户端管理 SSH transport。VS Code Remote Agent Sessions 也直接支持 SSH，复用现有远程主机访问能力。

优点：

- 服务器 Gateway 可以继续只监听 loopback；
- 大多数自托管服务器已经具备 SSH；
- 可以复用 `~/.ssh/config`、ProxyJump 和 SSH Agent。

代价：

- daemon 需要管理端口分配、host key 校验、断线重连和 SSH 子进程生命周期；
- SSH 认证成功不等于 Hermes API 鉴权成功，Bearer Token 仍然必须保留；
- 移动端和未来 Web 入口不一定具备 SSH 环境。

### 公网 HTTPS/WSS

Hermes 官方允许远程后端通过反向代理和路径前缀访问；对于公网暴露，推荐 OAuth/OIDC。VS Code Dev Tunnel 则代表另一种带账户认证的托管中继模式，并明确禁止匿名 tunnel 用于 Agent 控制。

优点：

- 任何网络位置都可以连接；
- 更适合未来多设备和 Web 入口。

代价：

- TLS、域名、反向代理、身份提供商和安全维护成本最高；
- Agent API 具备终端和文件工具，错误暴露的风险显著高于普通只读服务。

## 对 Agent Ferry 的建议

V0.1 使用 Hermes API Server 的 Runs API 作为首选控制接口，Hermes Connection 由以下两层组成：

```text
HermesConnection
├── endpoint
│   ├── base_url
│   ├── profile_or_model
│   └── api_key_ref
└── transport
    ├── direct
    └── ssh_tunnel
        ├── ssh_host
        └── remote_host / remote_port
```

推荐的产品优先级：

1. `direct`：支持 Tailscale、WireGuard、可信局域网或用户自行配置的 HTTPS Endpoint；
2. `ssh_tunnel`：作为 daemon 内建回退，复用 `~/.ssh/config`，Gateway 可保持 loopback；
3. OAuth/OIDC 公网登录：等多设备或 Web 入口进入范围后再支持；
4. Agent Ferry 自建云中继：不进入当前产品范围。

无论采用哪种传输，均保留 Hermes Bearer Token；凭据只以操作系统安全存储中的引用出现在配置里。连接建立后先请求 `/v1/capabilities`，再根据服务器实际支持情况启用任务状态、SSE、审批和取消。

## 参考资料

- [Hermes Programmatic Integration](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)
- [Hermes API Server](https://hermes-agent.nousresearch.com/docs/user-guide/features/api-server/)
- [Hermes Desktop Remote Backend](https://hermes-agent.nousresearch.com/docs/user-guide/desktop)
- [OpenClaw Remote Access](https://docs.openclaw.ai/gateway/remote)
- [VS Code Remote Agent Sessions](https://code.visualstudio.com/docs/agents/remote-agent-sessions)
