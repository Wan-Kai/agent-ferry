# Agent Ferry

Agent Ferry 是一个本地优先的浏览器到 AI Agent 的内容交接工具。

它通过 Chrome 扩展提取网页内容，由本地 Rust 程序将内容保存到用户选择的工作区，并在 Claude Code、Codex、OpenCode、Pi 或 Hermes 中开始后续工作。

项目尚处于初始化阶段。产品范围、架构决策和路线图见 [READE_DEV.md](./READE_DEV.md)。

## 当前目录

```text
crates/
  agent-ferry-core/      领域模型与核心流程
  agent-ferry-protocol/  Chrome Native Messaging 协议
  agent-ferry-cli/       aferry 命令行程序
  agent-ferry-host/      agentferry-host Native Messaging Host
extension/               WXT + React + TypeScript Chrome 扩展
docs/adr/                架构决策记录
```

## 开发

```bash
cargo test --workspace

cd extension
npm install
npm run compile
```

普通用户未来使用预编译产物，不需要安装 Rust 或 Node.js。
