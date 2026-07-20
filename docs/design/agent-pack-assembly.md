# Agent Pack 组装方案（ACP 后续阶段）

> 状态：Draft
> 事实来源：ADR 0018、ADR 0020 与后续 ACP 方向
> 范围：未来 Agent Pack，不描述当前 V0.1 运行路径

根据 ADR 0026，V0.1 的 Claude Code 直接使用 Print Mode，不安装 Agent Pack 或 ACP adapter。本文保留为后续 Managed Session 和其他 Agent 接入的顶层设计，不属于首版实现范围。

## 设计目标

Agent Ferry Core 保持轻量、稳定，并能按需接入不同语言和分发形态的 Agent。新增 Agent 不应要求把第三方代码链接进 daemon，也不应为所有用户增加下载体积。

## 核心思路

Agent Pack 不是动态链接库，也不是任意代码插件。它是一个经过签名的声明式安装单元，描述：

- 需要哪些版本化组件；
- 支持哪些平台；
- 如何解析启动命令；
- 使用哪种协议；
- 如何进行无 token 消耗的健康检查；
- 哪些配置和认证由 Agent 自己管理。

Core 只实现三种稳定执行后端：

```text
LocalAcpProcess   ACP over stdio
RemoteHermes      HTTP Runs API + SSE
ExternalNative    解析用户已安装的原生 ACP 命令
```

本地 Pack 最终都解析为 `LocalAcpProcess`。OpenCode、Hermes 等用户已有命令通过 `ExternalNative` 解析后也进入相同 ACP Client；云端 Hermes 保留独立 HTTP Controller。

## 组件、Pack 与安装实例

将 Pack 拆成三层，避免重复携带 runtime：

### Component

可复用、不可变、按版本寻址的文件集合，例如：

- `runtime.node@22.18.0-darwin-arm64`；
- `adapter.claude-acp@0.59.0`；
- `adapter.codex-acp@1.1.3-darwin-arm64`。

Component 安装后只读，以 SHA-256 和签名校验。相同 Component 只保存一份。

### Agent Pack

面向用户的组合定义，例如 `claude@2026.07`。它本身很小，只包含 manifest 和 lock，引用精确 Component：

```json
{
  "schema_version": 1,
  "id": "claude",
  "version": "2026.07",
  "platforms": ["darwin-arm64"],
  "protocol": "acp-stdio",
  "components": [
    "runtime.node@22.18.0-darwin-arm64",
    "adapter.claude-acp@0.59.0"
  ],
  "launch": {
    "executable": "${component:runtime.node}/bin/node",
    "args": ["${component:adapter.claude-acp}/dist/index.js"]
  },
  "probe": {
    "kind": "acp_initialize",
    "timeout_ms": 10000
  }
}
```

Manifest 不允许 shell 字符串，只允许结构化 executable、args 和受控环境变量映射，避免命令注入。

### Installed Agent

用户启用后的本地配置，只保存 Pack 版本、Workspace、用户可编辑设置和认证状态引用。它不复制 Pack 文件。

## 目录结构

macOS 建议使用：

```text
~/.local/share/agent-ferry/
└── agent-packs/
    ├── components/
    │   ├── runtime.node/22.18.0/darwin-arm64/
    │   ├── adapter.claude-acp/0.59.0/
    │   └── adapter.codex-acp/1.1.3/darwin-arm64/
    └── packs/
        ├── claude/2026.07/pack.json
        └── codex/2026.07/pack.json

~/.agent-ferry/
└── agent-packs/
    ├── active/
    │   ├── claude.json
    │   └── codex.json
    └── state/
        └── agent-ferry.sqlite3
```

不可变组件与 Ferry 版本化程序共用 `~/.local/share` 生命周期，用户选择和状态保存在
`~/.agent-ferry`。临时下载和解压位于目标安装目录同一文件系统的 staging 目录，校验成功后
通过原子 rename 激活；日志继续使用平台日志目录。

## 安装事务

```text
用户选择 Agent
→ Core 获取签名 Pack manifest
→ 计算缺失 Component 与增量体积
→ UI 展示来源、版本、下载量和磁盘占用
→ 用户确认
→ 下载到 staging
→ 校验 hash、签名、平台和允许的文件类型
→ 原子安装 Component
→ 执行 ACP initialize probe
→ 写入 active pointer
→ Agent 可用
```

任何步骤失败都不修改当前 active 版本。升级先并行安装新版本，probe 成功后切换 pointer；旧版本保留一个回滚窗口，再由垃圾回收删除。

## CLI 入口

Pack Manager 是 Core 用例，`aferry` 是 V0.1 唯一的软件安装入口。`aferry setup` 只读取环境、展示状态和生成下一步命令；它不会下载或安装 Agent。daemon 和浏览器扩展也只能查询安装状态，不能在后台静默安装。

首次运行：

```text
$ aferry setup

Agent Ferry Core              ready
Chrome Native Host            ready
Chrome Extension              not detected

Targets
cloud-hermes                  needs connection
claude-code                   missing prerequisite
  Claude Code                 not installed
  Claude Code ACP adapter     not evaluated
opencode                      detected, not enabled
pi                            supported later

Next actions
  aferry connection add hermes
  Install Claude Code from its official distribution
  aferry agent enable opencode
```

常用命令：

```text
aferry agent list [--json]
aferry agent enable <id> [--command <absolute-path>]
aferry agent disable <id>
aferry agent doctor [id] [--json]
aferry adapter list [--available] [--json]
aferry adapter install <adapter-id> [--version <version>] [--yes]
aferry adapter update [adapter-id]
aferry adapter remove <adapter-id>
aferry connection add hermes
aferry connection list
aferry connection doctor <id>
```

状态使用稳定枚举，展示文案可以变化：

```text
available
missing_prerequisite
needs_adapter
needs_authentication
needs_selection
installing
installed
detected
enabled
needs_configuration
incompatible
broken
update_available
```

`adapter install` 在确认前展示 Pack、增量下载量、安装后占用、来源和包含组件。确认后进入既定的 staging、签名校验、原子安装和 ACP probe 流程。probe 失败时保留诊断信息，但不启用新版本。

这里的安装对象只限 Ferry 管理的 adapter、runtime 和 Pack，不包括 Claude Code 等第三方宿主 Agent。目标诊断使用两层状态：宿主缺失时只显示官方安装指引；检测到兼容宿主后，才允许用户显式执行 `aferry adapter install claude-code-acp`。用户界面仍只展示一个 `Claude Code` 目标，并在详情中展开宿主与 ACP adapter。

Ferry 只用可执行文件和版本命令判断宿主是否安装，不读取宿主 token 或凭据文件。adapter probe 报告未认证时，目标进入 `needs_authentication`，并引导用户回到宿主产品完成登录后重新运行 doctor。

宿主探测结果必须解析为绝对可执行路径。单一兼容候选可以自动绑定；存在多个候选时进入 `needs_selection`，由用户通过 `agent enable --command <absolute-path>` 选择。已绑定目标不会因为 `PATH` 顺序变化而静默切换，且 `--command` 不接受 shell 命令或附加参数。

每次创建会话前先检查已绑定宿主的路径、权限和文件身份。文件未变化时复用最近一次兼容性结论；发生变化时重新读取版本并运行 ACP probe。不兼容时阻止任务并提示显式更新 adapter，不能在任务启动路径中自动下载。

## 运行时组装

daemon 不执行 manifest 中的任意脚本。Pack Manager 将 manifest 解析为内部 `LaunchSpec`：

```text
LaunchSpec
├── executable: absolute path
├── args: string[]
├── cwd: Workspace root
├── env_allowlist
├── protocol: acp-stdio
├── startup_timeout
└── pack_identity
```

启动时：

1. 根据 active pointer 解析不可变 Pack；
2. 验证 Component 仍存在且 hash 未变化；
3. 构造清理过的环境变量，不继承危险的 Node/Python 注入变量；
4. 启动子进程，stdout 只用于 ACP，stderr 进入脱敏日志；
5. 使用官方 Rust ACP SDK 完成 `initialize` 和 capability 协商；
6. 将 ACP 事件归一化写入 SQLite，再增量推送给扩展；
7. 子进程退出后记录明确原因，不在无限循环中盲目重启。

## 四类 Agent 如何装配

### Claude

Claude Code 是用户管理的宿主 prerequisite，Ferry 不负责安装。检测到兼容的 Claude Code 后，用户可以显式安装 Claude ACP Pack；该 Pack 引用共享 Node runtime 和固定 `claude-agent-acp` Component。Agent SDK 的平台 payload 作为 adapter Component 的锁定依赖，不使用全局 npm。

### Codex

优先使用官方 standalone `codex-acp` Component，不需要 Node；若实测 standalone 不完整，再由 Pack 引用共享 Node runtime。用户现有 `~/.codex` 认证和配置继续由 Codex 管理。

### OpenCode / 本地 Hermes

不下载 Pack runtime。External Native Descriptor 只记录命令探测规则，例如 `opencode acp` 或 `hermes acp`，解析为绝对路径后执行 capability probe。用户可以在高级设置中选择其他绝对路径，但不能填写 shell 命令。

### 云端 Hermes

不进入 Pack Manager。Hermes Connection 解析为 Remote Controller，包含 Endpoint、Transport 和 Keychain credential reference。

## Catalog 与供应链

Core 内置最小的受信任发布公钥和 Catalog Endpoint，不内置所有 Pack manifest。Catalog 返回签名的版本索引和 Component 元数据。

首版建议：

- Catalog 与 Component 通过 GitHub Releases 分发；
- manifest 和 lock 纳入 Agent Ferry 仓库评审；
- 组件 hash 固定，不允许同版本覆盖；
- 只接受 Agent Ferry 发布密钥签名的 Pack；
- 支持导入本地 Pack 文件用于开发，但必须显式开启开发模式；
- 不运行安装后脚本，不执行包管理器 lifecycle script。

## 体积控制

发布流程为 Core 和每个 Component 设定体积预算：

```text
core_download_bytes
core_installed_bytes
pack_incremental_download_bytes
pack_incremental_installed_bytes
shared_runtime_reuse_bytes
```

CI 对比基线并在超预算时失败或要求显式豁免。新能力若只服务特定内容类型或 Agent，默认拆为可选 Component。

## 首版可简化的部分

V0.1 不必实现通用第三方插件市场，但应保留上述边界：

- Catalog 只列官方验证 Pack；
- Component 类型先支持 `runtime` 和 `adapter`；
- 只支持 macOS Apple Silicon；
- 进入 ACP 阶段后再选择首个官方 Pack，不在 V0.1 实现 Claude Pack；
- Codex/Pi Pack 后续加入；
- External Native 先不提供任意用户 Pack，只允许内置的安全描述器。
- Agent 安装和连接配置只通过 `aferry` CLI 暴露，浏览器扩展只显示可用目标。

这样首版实现量可控，同时不会把一次性的 Claude 安装逻辑写死进 daemon。
