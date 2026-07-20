# Agent Ferry Privacy Policy

Effective date: July 20, 2026  
Product version covered: 0.1.x

Agent Ferry is a local-first browser extension and macOS command-line companion that sends a web page you choose to an AI Agent you choose. This policy describes the data handled by the Chrome extension, `agentferryd`, and the Native Messaging Host distributed as Agent Ferry.

## Data Agent Ferry handles

Agent Ferry handles the following data only to provide the page-to-Agent handoff requested by the user:

- website content and browsing context from the active tab, including the URL, title, visible article or thread text, author, publication date, site name, and extraction metadata;
- user-generated task instructions and Prompt templates;
- the selected Agent product, remote connection, or local Workspace;
- local task status, source metadata, bounded Agent output, and non-sensitive error information;
- authentication references required for a user-configured remote Hermes connection. The secret itself is stored in macOS Keychain and is not exposed to the Chrome extension.

Agent Ferry does not read a page in the background. Page extraction starts only after the user opens the extension, reviews the visible destination and Prompt, and clicks the start button. The button action is the user's instruction to process the displayed page through the displayed destination.

## How data is used and transferred

Captured page content and the visible Prompt are transferred through Chrome Native Messaging to the Agent Ferry process on the same Mac. Agent Ferry then sends them only to the destination selected by the user:

- a locally installed Claude Code, Codex, or OpenCode process running as the current macOS user; or
- a Hermes server explicitly configured by the user, using HTTPS directly or an SSH tunnel.

Agent Ferry does not operate a developer-owned service that receives captured pages, Prompts, task outputs, credentials, analytics, or browsing history. It does not sell user data, use it for advertising, credit decisions, or unrelated profiling, and does not permit its maintainers to read that data. Agent Ferry's use of information received from Chrome APIs complies with the Chrome Web Store User Data Policy, including the Limited Use requirements.

The selected Agent, its model provider, and a user-operated Hermes deployment may process or retain the submitted material according to their own configuration and policies. Agent Ferry does not control those systems. Users should choose a destination appropriate for the sensitivity of the page.

## Local storage and retention

- The Chrome extension stores Prompt templates, selected products and locations, and pinned task identifiers in Chrome local extension storage.
- `agentferryd` stores up to 500 task-history records under `~/.agent-ferry`. A record includes the Prompt, source metadata, target, state, bounded output, and errors, but not the captured page body or credentials.
- For local Agents, the captured page body is written to a user-private temporary Markdown Artifact. Expired Artifacts are removed when later Artifacts are created, and `aferry uninstall` removes the current Artifact directory. A host Agent may independently retain its own session or files.
- Hermes secrets are stored in macOS Keychain. Configuration files contain credential references, not the secret value.
- Agent Ferry diagnostic logs are designed not to contain captured page bodies, Prompts, or credentials.

## Security

Agent Ferry requests only `activeTab`, `scripting`, `nativeMessaging`, and `storage` permissions. `activeTab` grants temporary access only to the page on which the user invokes the extension. The extension does not request persistent access to all websites.

The extension-to-native transfer remains on the same computer. Remote Hermes connections require HTTPS; plain HTTP is accepted only for loopback addresses. SSH connections forward the remote service to a local loopback address. Local configuration, history, IPC endpoints, temporary Artifacts, and credentials use operating-system access controls appropriate to their role.

## User control and deletion

Users can edit or delete Prompt templates and delete completed task records from the extension. Removing the Chrome extension clears its Chrome-managed local storage according to Chrome behavior.

The following command removes the program but preserves recoverable settings, logs, and Keychain credentials:

```bash
aferry uninstall
```

The following explicit command also deletes Agent Ferry configuration, history, logs, and referenced Hermes credentials:

```bash
aferry uninstall --purge --yes
```

Data already sent to a selected local Agent, model provider, or Hermes server must be managed or deleted in that destination.

## Changes and contact

Material privacy changes will be reflected in this document and in the Chrome Web Store disclosure before the corresponding extension update is published.

Questions or requests can be filed through the [Agent Ferry issue tracker](https://github.com/Wan-Kai/agent-ferry/issues). Do not include private page content, credentials, or other sensitive information in a public issue.

---

## 中文摘要

Agent Ferry 只在用户查看了当前页面、Agent 运行位置和完整 Prompt，并点击开始按钮后，提取当前页正文并交给用户选择的本地 Agent 或用户自己配置的 Hermes。项目维护者不运营接收正文、Prompt、输出、凭据或浏览历史的服务器，也不将数据用于广告、出售、信用判断或无关画像。

扩展本地保存 Prompt 模板、选择项和关注的任务编号；daemon 本地保存有界任务历史，但不把网页正文或凭据写入历史。远程 Hermes 凭据保存在 macOS Keychain。普通卸载会清除临时网页正文；`aferry uninstall --purge --yes` 还会删除 Agent Ferry 配置、历史、日志和所引用的 Hermes 凭据。已经交给宿主 Agent、模型提供方或 Hermes 的数据，需要在相应系统中管理。
