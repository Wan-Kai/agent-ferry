# ADR 0023：宿主版本漂移与 Adapter 兼容性检查

## 状态

已接受，2026-07-15。

V0.1 只应用宿主文件身份与 Print Mode flags 检查；adapter 兼容部分延期到 ACP 阶段。

## 背景

Claude Code 由用户独立升级，Ferry 管理的 ACP adapter 可能仍绑定旧的兼容范围。每次任务都运行完整 doctor 会增加启动延迟，但完全不检查又可能在版本变化后启动一个已知不兼容的组合。

## 决策

1. 绑定宿主可执行文件时记录绝对路径、文件身份/指纹和已验证版本；
2. 每次新建会话前先执行低成本的路径、可执行权限和文件身份检查；
3. 文件身份未变化时复用最近一次兼容性结果，不重复运行版本命令或完整 probe；
4. 文件发生变化时重新读取宿主版本；V0.1 检查 Print Mode 所需 flags，后续 ACP 阶段再执行 initialize/capability probe；
5. Catalog/adapter manifest 必须声明可解释的宿主版本兼容范围；
6. 发现不兼容时阻止创建任务，目标进入 `incompatible`，并提示用户显式执行 `aferry adapter update claude-code-acp`；
7. Ferry 不因版本漂移自动下载、安装或升级 adapter；
8. 更新后的 adapter 只有通过完整 probe 才能切换为 active，失败时继续保留旧版本和诊断信息。

## 后果

- 常规任务只增加一次低成本文件检查；
- 宿主升级后的第一次任务可能因重新 probe 稍慢或被阻止；
- Pack 发布流程需要维护宿主兼容矩阵；
- 文件身份的具体实现按平台封装，不能只依赖容易碰撞的文件名或 `PATH`。
