# ADR 0031：采用 CLI 风格的安装与数据目录

## 状态

Superseded，2026-07-20。程序安装与发行渠道由 [ADR 0032](./0032-homebrew-distribution-without-developer-id.md)
取代；用户数据、日志和系统规定目录的结论在新 ADR 中继续成立。

## 背景

Agent Ferry 的核心交付物是 `aferry`、`agentferryd` 和 `agentferry-host` 三个命令行程序，不是放入 `/Applications` 的图形应用。早期开发版本把配置、历史和运行状态统一放在 `~/Library/Application Support/Agent Ferry`，这会让程序版本、可执行入口和用户数据的生命周期混在一起，也不符合 Claude Code 等 CLI 工具的安装习惯。

首发还需要支持无管理员权限的一行安装、版本原子切换、失败回滚和默认保留用户数据的卸载。安装布局必须先稳定，后续 LaunchAgent、升级器和卸载器才能共享同一契约。

## 决策

1. 版本化程序安装到 `~/.local/share/agent-ferry/versions/<version>`，`current` 符号链接指向当前版本；
2. `~/.local/bin/aferry`、`agentferryd` 和 `agentferry-host` 只作为指向 `current/bin` 的命令入口；
3. 配置、历史、缓存和运行状态保存在权限为 `0700` 的 `~/.agent-ferry`；
4. `AGENT_FERRY_HOME` 继续作为测试和显式开发隔离入口，不参与普通用户安装；
5. LaunchAgent、日志和 Chrome Native Messaging manifest 分别使用 macOS 与 Chrome 规定的目录，不能搬入 Ferry 数据目录；
6. 普通安装不写 `/Applications`、`/usr/local` 或系统目录，也不要求 `sudo`；
7. 旧数据只在 daemon 停止后通过 `aferry data migrate` 迁移。旧目录存在且新目录不存在时使用同文件系统 rename；两边同时存在或任一路径是符号链接、普通文件时拒绝操作；
8. `setup` 和 `doctor` 保持只读，正式安装器负责在启动新 daemon 前调用迁移命令。

## 备选方案

### 全部继续放在 Application Support

不采用。该目录适合 macOS 应用数据，但不适合作为版本化 CLI 二进制和 PATH 入口；继续混放会增加升级和卸载误删用户数据的风险。

### 安装到 `/usr/local/bin`

不采用。部分机器需要管理员权限，也容易与 Homebrew、企业设备策略和其他安装来源冲突。

### 首发提供 `.pkg` 或 `.dmg`

暂不采用。图形安装器不能替代版本管理、升级回滚和卸载语义；首发先提供可审查的 HTTPS 安装脚本与签名、公证后的预编译二进制，后续可增加图形分发渠道。

## 后果

- 普通用户不需要 Rust、Node.js 或管理员权限；
- 程序升级与用户数据生命周期分离，回滚只切换 `current`；
- `~/.local/bin` 未进入 PATH 时，安装器必须给出明确修复提示；
- 旧开发版本不能靠首次启动静默迁移，安装器必须先停止旧 daemon；
- Native Host manifest 和 LaunchAgent 的清理由各自管理命令负责，不能通过删除 `~/.agent-ferry` 代替。
