# ADR 0021：宿主 Agent 的认证边界

## 状态

已接受，2026-07-15。

## 背景

Ferry 需要判断本地 Agent 是否能通过受支持的控制路径工作，但 Claude Code 等宿主 Agent 的账号、token、订阅和登录状态属于宿主产品自身管理范围。直接读取其凭据文件会扩大 Ferry 的权限和安全责任，也会把 Ferry 绑定到第三方未承诺稳定的内部存储格式。

## 决策

1. Ferry 通过可执行文件和版本命令判断 Claude Code 是否已安装及是否兼容；
2. Ferry 不读取、复制、导入或保存 Claude Code 的 token、cookie、credential 文件或账号信息；
3. V0.1 通过不使用工具的受限 Print Mode doctor 调用判断目标能否工作；后续 ACP 阶段再使用 initialize/capability probe；
4. probe 表明未认证时，目标进入稳定状态 `needs_authentication`；
5. CLI 和浏览器只提供 Claude Code 官方认证流程的指引，用户在宿主产品中完成登录后执行 `aferry agent doctor claude-code` 重新检查；
6. Ferry 自己管理的云端 Hermes Bearer token 仍保存到系统 Keychain，因为它是 Ferry Connection 的显式配置，不属于本决策中的宿主凭据。

## 后果

- Ferry 无法主动展示 Claude 账号详情或修复宿主登录问题；
- V0.1 doctor 需要区分 `missing_prerequisite`、`incompatible`、`needs_authentication` 和 `ready`；后续 ACP 阶段再增加 `needs_adapter`；
- 日志必须避免记录 adapter stderr 中可能出现的认证信息；
- 宿主认证格式变化不会迫使 Ferry 读取或迁移凭据文件。
