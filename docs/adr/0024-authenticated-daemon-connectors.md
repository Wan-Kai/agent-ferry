# ADR 0024：可鉴权的 Daemon Connector 边界

## 状态

已接受，2026-07-15。

## 背景

V0.1 只有 Chrome 扩展需要连接本地 daemon，最小路径是 Chrome Native Messaging Host 到 Unix Domain Socket。未来可能增加其他本地应用、网页或云端连接方；如果业务 API 直接绑定 Native Messaging 或把“本机连接”视为天然可信，后续很难安全扩展。

## 决策

1. V0.1 只实现 `ChromeNativeHostConnector`，不开放 HTTP、WebSocket、TCP 或远程监听端口；
2. daemon 内部将传输、连接方认证和业务命令分层，业务处理只接收已经认证的 `Principal`；
3. Native Messaging manifest 只允许正式扩展 ID，Native Host 验证由受信任 Chrome 启动；
4. Native Host 通过当前用户私有的 Unix Domain Socket 连接 daemon，daemon 校验 peer UID、进程身份和 Agent Ferry 代码签名；
5. 所有输入使用有版本的结构化消息和白名单命令，连接方不能提交任意 shell 命令；
6. 未来 Connector 必须定义自己的身份建立、凭据轮换、撤销、重放保护和权限策略，不能复用“本机同 UID”作为远程认证；
7. 新增 Connector 不得绕过同一套任务、Workspace、ACP 权限和审计规则；
8. V0.1 只保留 Connector/Auth 接口和 Principal 数据模型，不实现未来传输或配对界面。

## 后果

- 首版没有本地 Web 服务及其端口攻击面；
- Native Host 不是业务逻辑容器，只负责协议桥接和身份建立；
- daemon API 从第一版起需要显式协议版本和连接方身份；
- 未来支持网页或云端接入时必须新增安全评审，不能仅打开一个监听端口；
- macOS 代码签名验证需要在开发构建中提供明确的开发模式，但正式版不得静默降级。
