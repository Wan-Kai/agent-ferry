# Commit 与产物证据契约

> 状态：Current
> 事实来源：`scripts/verify` 和发布产物边界
> 范围：离线验证、RC 和真实环境 Case 的证据字段

## 最小字段

- 完整 Git commit 与工作区是否干净；
- UTC 执行时间；
- Rust、Cargo、Node 与 npm 版本；
- 实际执行的统一验证命令和结果；
- Extension ZIP、CLI、daemon、Native Host 等被验收产物的 SHA256；
- 正式 macOS 发布的 Developer ID Team、两个架构 Notary submission ID、无 issue log 与在线 Gatekeeper 结论；
- 真实环境 Case 的非敏感结果引用。

普通开发可以在脏工作区运行验证，证据中的 `worktree_clean` 必须为 `false`，且不能作为 RC 证据。RC 证据必须通过 `--require-clean` 生成。提交之后若代码、锁文件、构建配置或产物改变，原证据不能沿用。

证据文件不得包含环境变量、命令完整输出、Token、Cookie、用户目录内容或捕获正文。CI 应把证据作为对应 commit 的构建产物保存，而不是把自引用 commit 写回同一个提交。
