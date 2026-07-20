# ADR 0008：首个可用版本只正式支持 macOS 客户端

## 状态

已接受，2026-07-14。

## 背景

本地 `agentferryd` 涉及操作系统服务注册、本地 IPC、终端启动和凭据安全存储。若首个纵向闭环同时实现 macOS、Linux 和 Windows，会把大量工作投入平台适配，而不是验证浏览器到本地 Claude Code 和远程 Hermes 的核心价值。

用户当前在 macOS 上运行浏览器、本地 daemon 和 Claude Code；远程 Hermes 运行在 Linux 服务器，不要求 Linux 桌面客户端。

## 决策

首个可用版本只正式支持 macOS 客户端，包括：

- Chrome 扩展和 Native Messaging Host 注册；
- `agentferryd` 的 launchd 安装与生命周期；
- macOS 本地 IPC 和安全凭据存储；
- 可见终端中的本地 Claude Code 启动；
- 从 macOS 连接 Linux 服务器上的 Hermes Gateway。

核心领域模型、协议和目标适配接口保持跨平台，不在业务层引入不必要的 macOS 假设。Linux 和 Windows 客户端在 macOS 纵向闭环稳定后补充。

## 备选方案

### 首版同时支持所有桌面平台

不采用。它会同时引入 systemd/launchd/Windows Service、不同 IPC、终端和凭据后端，扩大验证范围。

## 后果

- CI 仍可在其他平台执行与操作系统无关的核心测试；
- V0.1 的安装文档和验收以 macOS 为准；
- Linux 和 Windows 发布产物不属于首个可用版本承诺；
- 平台相关实现必须位于明确适配层，避免未来移植时重写核心流程。
