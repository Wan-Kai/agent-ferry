# Architecture Decision Records

> 状态：Current
> 事实来源：本目录 ADR 与文档生命周期规则
> 范围：长期架构决策的状态、编号和替代关系

当设计决策会影响多个组件、兼容性或长期维护成本时，在此目录新增 ADR。每份 ADR 至少记录背景、决策、备选方案、后果和当前状态。

状态使用 Accepted、Proposed 或 Superseded。改变已接受决策时新增 ADR，旧 ADR 保留原文并链接替代者，不直接把历史背景改写成新结论。

## 当前补充决策

- [ADR 0030：保存有界的本地任务历史](./0030-bounded-local-task-history.md)，取代 ADR 0028、0029 中禁止 Ferry 历史的部分。
- [ADR 0033：使用原生 Homebrew Bottle 分发预编译 Core](./0033-native-homebrew-bottles.md)，已由 ADR 0034 取代。
- [ADR 0034：Homebrew 安装与用户级激活分离](./0034-explicit-homebrew-activation.md)，保留原生 Bottle，并用显式 CLI 命令完成 LaunchAgent 与 Native Host 激活。
