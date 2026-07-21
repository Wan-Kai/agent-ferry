# ADR 0033：使用原生 Homebrew Bottle 分发预编译 Core

## 状态

Accepted，2026-07-21。取代 [ADR 0032](./0032-homebrew-distribution-without-developer-id.md) 中把普通
双架构归档直接作为 Formula source 的发行形式；保留 Homebrew 渠道、无 Developer ID、用户数据与
卸载边界。

## 背景

ADR 0032 已经避免在用户机器上编译 Rust，但普通 `url` 归档仍会进入 Homebrew 的 source install
路径。Homebrew 会在解压预编译二进制前执行开发工具完整性检查；当用户没有 CLT，或系统升级后 CLT
收据与 SDK 内容不一致时，安装会在 Agent Ferry 代码运行前失败。

Agent Ferry 的 Core 已经由 CI 生成固定架构、固定最低部署版本的 Mach-O，没有理由让普通安装依赖
source install。Homebrew 官方 Bottle 同时提供架构/系统选择、SHA-256、keg 收据和包管理器原生的
升级语义。

## 决策

1. 用户入口仍为 `brew install Wan-Kai/tap/agent-ferry`；
2. 发布流水线先生成 ad-hoc 签名的双架构 source fallback 归档，再在原生 GitHub macOS runner 上用
   `brew install --build-bottle` 与 `brew bottle` 生成真正的 Bottle；
3. Apple Silicon Bottle 在 `macos-14` 生成，tag 为 `arm64_sonoma`。Homebrew 可以在更新 macOS 上
   选择同架构的旧系统 Bottle，因此覆盖当前 Apple Silicon 支持面；
4. Intel Bottle 在标准 `macos-15-intel` runner 生成，tag 为 `sequoia`。Intel Sonoma 没有可用的
  标准原生 runner，暂时保留预编译 source fallback；它不会编译 Rust，但仍受 Homebrew CLT 检查；
5. Bottle 必须由 Homebrew 从已安装 keg 生成，归档中包含 Formula、`INSTALL_RECEIPT.json` 和三个
   Core 二进制；禁止把普通 tar 改名为 Bottle；
6. 最终 Formula 固定 Bottle root URL、tag、SHA-256 与 `cellar: :any_skip_relocation`。source URL
   继续固定双架构归档与 SHA-256，只用于没有匹配 Bottle 的兼容回退；
7. RC 在发布前使用本地 Bottle URL 执行 `brew fetch --force-bottle` 和真实 `brew install`，并从
   “Pouring bottle”日志、服务、Native Host、日志和双阶段卸载验证用户可观察行为；
8. GitHub Release、checksums、Artifact Attestation 和验证证据同时覆盖 Bottle、fallback 归档、
   Formula 与 Chrome ZIP。

## 备选方案

### 继续使用普通预编译归档 Formula

不采用为默认路径。它不会编译 Rust，但仍被 Homebrew 当作 source install，无法满足“已有 Homebrew
即可安装”的体验。

### 手工拼装或重命名 Bottle

不采用。文件名和 SHA 正确也不能替代 keg 布局、安装回执、内嵌 Formula 与 Homebrew 自身的打包
契约，容易在升级或新 Homebrew 版本中失效。

### 只发布 Apple Silicon

不采用。Intel 用户仍在首发支持面；没有匹配 Bottle 的旧系统可以明确退回现有预编译归档，而不是
静默删除支持。

## 后果

- Apple Silicon Sonoma 及更新系统、Intel Sequoia 及更新系统的普通安装不再进入 CLT source
  sanity check；
- 用户仍不需要 Rust、Node.js、Apple Developer 账号或 Agent runtime；
- 发布流水线增加两个原生 macOS runner job，耗时与 CI 成本增加，但换来可独立验收的架构产物；
- Intel Sonoma 暂时仍可能要求健康的 CLT；获得可用的原生 Sonoma Intel runner 后再补对应 Bottle，
  不通过伪造 tag 扩大兼容范围；
- Bottle 仍只有 ad-hoc 签名且未经 Apple notarization，不能把 Homebrew 完整性验证描述为 Apple 身份
  背书。
