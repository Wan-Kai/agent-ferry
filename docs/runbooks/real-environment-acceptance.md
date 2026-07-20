# 真实环境延迟验收

> 状态：Current
> 事实来源：产品外部边界与 `docs/acceptance/case-template.md`
> 范围：无法由默认离线测试忠实证明的 Chrome、macOS 和远程集成

## 原则

`./scripts/verify` 通过只代表离线门禁通过，不能写成真实环境已经通过。真实环境暂不可用或未获副作用授权时，把 Case 标记为 `PENDING_ENV` 或 `BLOCKED_AUTHORIZATION`，保留阻塞原因后继续不依赖该结论的工作。

真实环境验证不得把 Token、Cookie、Keychain 内容、完整隐私正文或原始错误栈写入证据。需要外部引用时只保留非敏感 ID、脱敏摘要或本地证据文件路径。

## 建议 Case 集合

### Chrome 与 Native Messaging

- 使用生产构建或明确记录 commit 的 unpacked Extension。
- 全新安装只打开一次 onboarding；页面给出可复制的 Core 安装命令，安装前显示未连接，Core 安装后“重新检查”显示具体版本。
- 生产构建必须记录 Dashboard Item ID，并确认 ZIP manifest 的 `key` 推导结果、Homebrew Formula 的扩展 ID 与 Native Host `allowed_origins` 完全一致。
- 检查 manifest 只有 `activeTab`、`scripting`、`nativeMessaging`、`storage`，没有 `host_permissions`；未点击开始时不能读取页面正文。
- 发送页必须在按钮前展示将提取当前页正文、可见 Prompt 和所选目的地；切换本地/远程位置后披露同步更新。
- 验证普通页面、懒加载页面、X、arXiv HTML/PDF、空正文和超限错误。
- 验证 activeTab、Popup 关闭、重新打开、任务继续和历史展示。
- 验证 Extension → Native Host → daemon → fake/controlled Agent 的完整链路。

按照仓库约定优先使用真实 Chrome；Chrome 未启动时可以启动后继续验证。

### macOS 本机边界

- 通过 `brew install Wan-Kai/tap/agent-ferry` 完成无 sudo 安装；验证 Homebrew keg/opt、LaunchAgent 和 Chrome Native Host 各自落在约定路径，并核对 Formula SHA256 与 Artifact Attestation。
- 验证 `brew upgrade` 后 daemon 切换到新 keg；`aferry uninstall` 只清理运行资源，随后 `brew uninstall` 删除程序。
- 真实 Keychain create/read/delete smoke，输出中不出现 Secret。
- Native Host manifest、私有 Socket、文件权限和 daemon 生命周期。
- 已安装 Claude、Codex、OpenCode 的检测、认证提示和一次性任务。
- 取消/超时终止目标进程组，其他任务不受影响。
- 普通卸载保留配置、历史、日志和凭据，但删除临时网页正文；`--purge --yes` 再删除 Ferry 数据、日志和引用的 Hermes 凭据。

### Remote Hermes

- Direct URL 与 SSH Tunnel 分别验证 capability、鉴权、提交、SSE、断线和取消。
- 使用用户拥有且明确授权的非敏感环境；写入、取消等副作用逐 Case 获得授权。

## 证据绑定

每个已执行 Case 记录：完整 commit、Extension ZIP 与 Rust 二进制 SHA256（如适用）、执行时间、环境摘要、步骤、预期、实际结果和非敏感引用。批量发版前冻结 RC，所有 Case 必须针对同一 RC 或明确说明重跑范围。

本仓库暂不引入机器可读任务状态或强制批次状态机；Case 使用独立模板保存，避免把未执行项误写成通过。
