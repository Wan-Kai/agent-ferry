# Agent Ferry V0.1：浏览器到 Hermes / Claude Code 的实时交接

> 状态：Historical
> 事实来源：2026-07-15 的 V0.1 产品与测试基线
> 范围：早期实时交接范围；当前能力和验收方式见 Current 架构与 Runbook

## Problem Statement

用户在浏览器中阅读论文、Twitter/X 帖子、YouTube 内容、arXiv 论文或技术文档时，希望让自己已经在使用的 Agent 继续分析，但目前必须手工复制正文、切换应用、进入正确目录、启动 CLI、组织 Prompt，并在长内容、特殊站点和远程 Agent 之间重复处理不同的传递方式。

对本地 Claude Code，用户还需要确保 CLI 在正确 Workspace 中启动并获得完整网页上下文；对部署在服务器并连接 IM 的 Hermes，用户希望把完整内容提交给同一 profile，让 Hermes 自己决定如何保存为文件或记忆，以便后续在 IM 中召回。现有流程割裂、容易漏掉正文，并且不适合长文和频繁交接。

## Solution

提供一个 macOS 优先的 Agent Ferry V0.1：Chrome 扩展从当前页面提取实际内容，展示可选择和编辑的最终 Prompt，再通过 Chrome Native Messaging、薄 Native Host 和本地 `agentferryd` 把一次 Handoff 自动提交给唯一目标。

首批目标是云端 Hermes和本地 Claude Code，按云端 Hermes优先、本地 Claude Code 随后的顺序实现。Hermes 通过 Runs API 接收最终 Prompt、来源和完整 Markdown，并在支持时通过 SSE 实时返回输出；Claude Code 由 daemon 在选定 Workspace 中以一次性 Print Task 启动，通过 `stream-json` 实时返回输出。V0.1 不保存 Agent 回复历史、不支持中途续写或权限审批，也不引入 ACP；ACP、可靠会话、权限和 sandbox 在后续阶段独立设计。

## User Stories

1. As a browser reader, I want to hand the current page to an Agent without manually copying it, so that I can continue reading instead of assembling context by hand.
2. As a paper reader, I want the actual paper content sent rather than only its URL, so that the Agent can analyze the document immediately.
3. As a Twitter/X reader, I want thread content and source links preserved, so that the Agent understands the conversation structure.
4. As a YouTube viewer, I want available transcript text and video metadata captured, so that the Agent can analyze the content without watching the video itself.
5. As an arXiv reader, I want both HTML papers and PDFs handled appropriately, so that I can use the same Handoff action on either form.
6. As a reader of an ordinary website, I want lazy-loaded content fetched within safe limits before extraction, so that the captured Markdown is as complete as practical.
7. As a reader on a long or infinite page, I want capture limits to be explicit, so that the extension does not scroll forever or freeze the browser.
8. As a browser user, I want my scroll position restored after capture, so that using Agent Ferry does not disrupt where I was reading.
9. As a user, I want extraction failures and incomplete captures shown clearly, so that I do not unknowingly send an empty page or only a URL.
10. As a user, I want source title, URL, author, time, and extractor metadata retained when available, so that the Agent can cite and contextualize the material.
11. As a user, I want to configure reusable Prompt templates in the extension, so that common analysis tasks require less typing.
12. As a user, I want template selection to be optional, so that a one-click default remains available.
13. As a user, I want selecting a template to populate a single final Prompt editor, so that I can see exactly what task will be sent.
14. As a user, I want to edit the resolved Prompt for one Handoff without changing the template, so that I can customize individual tasks safely.
15. As a user, I want no hidden task instructions appended outside the visible final Prompt, so that the Agent's objective is transparent.
16. As a user, I want to select exactly one Handoff Target, so that a single click cannot accidentally start duplicate local and remote tasks.
17. As a user with multiple local projects, I want to configure multiple Workspaces, so that local Claude starts in the correct project context.
18. As a user, I want Workspace to be described as Claude's working directory rather than a sandbox, so that I understand its real security boundary.
19. As a user with remote Hermes, I want to configure a Hermes Connection without exposing remote server paths in the browser, so that the remote Agent retains ownership of its storage.
20. As a Hermes user, I want Direct URL connectivity preferred over an SSH tunnel, so that existing Tailscale, WireGuard, LAN, or HTTPS networking is reused.
21. As a Hermes user with SSH-only access, I want an SSH Tunnel fallback, so that I can connect without exposing a public control endpoint.
22. As a security-conscious user, I want Hermes Bearer Tokens stored in macOS Keychain, so that secrets do not pass through or remain in the extension.
23. As a Hermes user, I want the daemon to discover server capabilities before showing SSE or cancel controls, so that the UI never promises unsupported behavior.
24. As a Hermes user, I want the complete Markdown placed in a single Runs API input when accepted, so that Hermes receives coherent context.
25. As a Hermes user, I want oversized input to fail explicitly instead of being silently truncated, so that I know when the Agent did not receive the whole document.
26. As a Hermes user, I want Run output streamed live while my extension UI remains connected, so that I can observe the analysis as it happens.
27. As a Hermes user, I want a remote Run to continue when the browser UI closes, so that a long-running server task is not coupled to a popup lifecycle.
28. As a Hermes user, I want Hermes to decide whether and where to save submitted content, so that its existing files, memory, and IM recall remain authoritative.
29. As a Hermes user, I want later IM conversations to rely on Hermes's own persistence rather than a second Ferry knowledge base, so that information is not split across systems.
30. As a local Claude Code user, I want Ferry to detect my existing Claude installation, so that no duplicate Claude product is installed.
31. As a user without Claude Code, I want only official installation guidance, so that Ferry does not install or manage third-party Agent software.
32. As a user with one compatible Claude executable, I want it selected automatically by absolute path, so that setup is quick and deterministic.
33. As a user with multiple Claude installations, I want to choose the exact executable, so that Ferry never changes versions because PATH order changed.
34. As a security-conscious user, I want Ferry to avoid reading Claude tokens or credential files, so that Claude authentication remains owned by Claude Code.
35. As a Claude user, I want authentication problems reported through doctor output with native login guidance, so that I can fix them in Claude Code itself.
36. As a Claude user, I want every Handoff to create a new Print Task and Claude conversation, so that unrelated documents do not share context.
37. As a Claude user, I want the Print Task launched in my selected Workspace, so that CLAUDE.md and project configuration are discovered normally.
38. As a Claude user, I want Ferry to preserve my normal Claude Code configuration, Skills, hooks, and session behavior, so that automation behaves like my local CLI.
39. As a Claude user, I want the full Captured Content written to a temporary Artifact and referenced by absolute path, so that long pages do not overflow stdin or process arguments.
40. As a privacy-conscious user, I want the visible Prompt sent through stdin rather than argv, so that it is not exposed through a process list.
41. As a Claude user, I want V0.1 to run with the same host permissions as my local unrestricted Claude invocation, so that the task can finish without mid-run approval UI.
42. As a Claude user, I want it stated that unrestricted host execution is not a sandbox, so that I understand Claude can read, write, execute, and access the network with my user permissions.
43. As a Claude user, I want multiple Print Tasks to run concurrently even in the same Workspace, so that a long task does not block another Handoff.
44. As a Claude user, I want Ferry not to coordinate concurrent file or Git changes, so that the temporary V0.1 boundary is explicit rather than falsely safe.
45. As a user, I want local Claude output streamed live while the UI is connected, so that I can watch progress without opening a terminal.
46. As a user, I want a running task to continue after the extension UI closes, so that long tasks are not cancelled by browser focus changes.
47. As a user, I want Ferry to keep draining child-process output after UI disconnect, so that an unseen task does not deadlock on a full pipe.
48. As a user, I want output with no live subscriber discarded rather than stored, so that V0.1 does not introduce an unfinished history system.
49. As a user, I want to cancel a task while its live UI is connected, so that I can stop work I no longer need.
50. As a user, I want cancellation to terminate the whole Claude process group, so that child commands do not remain running after the parent exits.
51. As a user, I want cancelled partial output kept separate from a final answer, so that incomplete work is not presented as successful.
52. As a user, I want local tasks to time out after 60 minutes by default, so that a stuck unrestricted process does not run forever.
53. As a user, I want `timed_out`, `cancelled`, and `failed` reported as distinct terminal states, so that I understand why work stopped.
54. As a user, I want completed output available only in the currently connected V0.1 UI, so that the first release avoids premature result retention behavior.
55. As a user, I want Claude Code's own native session persistence left unchanged, so that Ferry does not alter or depend on Claude's internal history.
56. As a user, I want temporary Artifacts removed after a bounded retention period, so that captured pages do not accumulate indefinitely.
57. As a user, I want `aferry setup` to report Core, Native Host, extension, Agent, Workspace, and Connection readiness, so that I know what is usable.
58. As a user, I want setup and doctor commands to be read-only, so that diagnostics never install or upgrade software implicitly.
59. As a user, I want actionable next commands shown for missing configuration, so that setup failures are easy to resolve.
60. As a user, I want Chrome to be the only daemon Connector in V0.1, so that the initial local attack surface stays narrow.
61. As a security-conscious user, I want ordinary web pages unable to call the daemon directly, so that page JavaScript cannot launch host tasks.
62. As a security-conscious user, I want the Native Messaging manifest restricted to the official extension ID, so that unrelated extensions cannot invoke the bridge.
63. As a security-conscious user, I want the Native Host and daemon connected through a private Unix Domain Socket rather than a local TCP port, so that there is no web-accessible listener.
64. As a security-conscious user, I want the daemon to authorize structured Connector capabilities, so that the extension cannot install software, change Agent paths, or read secrets.
65. As a user, I want large Captured Content sent to the daemon in bounded frames, so that Native Messaging does not rely on an unbounded JSON payload.
66. As a user, I want each event associated with a task ID and sequence number, so that concurrent live streams cannot be confused.
67. As a user, I want the first release distributed as prebuilt macOS Apple Silicon artifacts, so that I do not need Rust or Node.js installed.
68. As a maintainer, I want optional Agent integrations excluded from Core, so that future targets do not inflate every user's download.
69. As a maintainer, I want ACP implemented later as a distinct Managed Session backend, so that the Print Task prototype is not stretched into an unreliable pseudo-session.
70. As a future ACP user, I want permissions, replay, attach, history, Side Panel, and sandbox designed together, so that those capabilities have coherent security and lifecycle semantics.

## Implementation Decisions

- Keep the existing Rust workspace and WXT/React extension as the implementation foundation, but replace placeholder domain assumptions that model Terminal/Managed/Background launch modes with the accepted V0.1 concepts: Handoff, Handoff Target, Workspace, Hermes Connection, Print Task, Live Result Stream, Artifact, and terminal task states.
- Add a long-running `agentferryd` process as the Handoff Hub. The daemon owns configuration, active task state, temporary Artifacts, local child processes, remote Runs, cancellation, timeout, and live event normalization.
- Keep `agentferry-host` as a thin Native Messaging Bridge. It handles Chrome framing and forwards versioned structured messages over a private Unix Domain Socket; it does not contain extraction, task, credential, or Agent logic.
- Use only `ChromeNativeHostConnector` in V0.1. Do not expose HTTP, WebSocket, TCP, or remote daemon listeners. Keep Transport, Connector Authentication, Principal, capability authorization, and business command handling as distinct internal layers so future Connectors can provide their own authentication.
- Restrict the Chrome Principal to submitting captures, reading available targets, creating tasks, receiving live task events, and cancelling a currently addressable task. Agent path changes, Hermes secrets, and daemon administration remain CLI-only operations.
- Version every protocol message. Associate every live event with task ID, monotonic sequence, timestamp, event type, and payload. Sequence is connection-local in V0.1 and is not a durable replay cursor.
- Transfer large Captured Content from extension to daemon in bounded chunks with explicit begin/chunk/end semantics and integrity checks. Do not rely on one unbounded Native Messaging JSON object.
- Implement a Content Extractor router in the extension. Use Defuddle for ordinary pages after bounded lazy-load triggering, specialized extraction for Twitter/X and YouTube, generic extraction for arXiv HTML, and a separate acquisition/text-extraction chain for arXiv PDF.
- Normalize extracted content to Markdown plus source metadata and extraction completeness. Empty or materially incomplete extraction is an explicit user-visible condition and never silently degrades to URL-only Handoff.
- Keep exactly one visible final Prompt editor. Prompt templates populate it, one-off edits do not mutate templates, and no hidden task intent is appended after user review.
- Model Handoff Target as a discriminated choice. Local Claude requires a Workspace; Remote Hermes requires a Hermes Connection. A Handoff contains one target only.
- Implement `RemoteHermesController` first. Prefer a user-supplied Direct URL, support SSH Tunnel fallback, use Bearer Token authentication in both cases, and store only the credential reference outside macOS Keychain.
- Discover Hermes capabilities before enabling SSE and cancellation. Submit effective Prompt, source metadata, and complete Markdown in one Runs API input. Reject server-size violations explicitly and do not silently truncate or introduce V0.1 upload chunking.
- Treat Hermes files, memory, summary, indexing, and IM recall as Hermes-owned behavior. Ferry never chooses a remote path or implements a parallel inbox or memory store.
- Implement `ClaudePrintController` after the Hermes path. It invokes the user-installed Claude Code executable directly; it never installs Claude Code or `claude-code-acp`.
- Resolve Claude candidates to absolute executable paths. Auto-bind one compatible candidate, require explicit selection when multiple candidates exist, and never switch a bound executable because PATH order changed.
- Detect Claude installation and supported Print Mode flags through version/help inspection. Diagnose authentication with a restricted no-tool Print Mode probe without reading Claude credential files.
- Create a temporary Markdown Artifact per local Handoff under the operating system temporary root. The Artifact is not written into the Workspace and is retained for 24 hours after terminal task state unless still running.
- Start Claude with the selected Workspace as cwd and structured argv equivalent to Print Mode, unrestricted permission mode, stream-json output, and verbose structured events. Write the visible Prompt, source information, and Artifact path to stdin and close it. Never construct a shell command string.
- Do not use bare mode or disable Claude's native session persistence. Preserve the user's login, Keychain, CLAUDE.md, Skills, hooks, plugins, project settings, and native session defaults; Ferry does not inspect or depend on Claude's native session history.
- V0.1 local execution is fixed to unrestricted host permissions. Workspace is contextual cwd, not an isolation boundary. Ferry adds no tool, file, shell, or network allowlist. Operating-system permissions and non-bypassable Claude/organization policies may still apply.
- Create a new task UUID and independent Claude conversation for every Handoff. Never use continue or resume. Allow tasks to run concurrently in the same Workspace and explicitly do not coordinate file, Git, or external side effects.
- Normalize both Controllers to the minimal lifecycle: starting, running, succeeded, failed, cancelled, and timed_out. There is no awaiting approval, pause, resume, product queue, or daemon-restart recovery in V0.1.
- Set a 60-minute default deadline for local Claude tasks. User cancellation and timeout terminate the full process group after a short graceful-exit window. Partial output from non-successful tasks is never marked final.
- Forward Claude stream-json and Hermes SSE output only to the currently connected extension UI. Do not persist response bodies, final answers, replay events, task history, notifications, or completion badges.
- UI disconnect never cancels a local or remote task. Continue draining local stdout/stderr to prevent pipe backpressure and discard output without a subscriber. V0.1 does not support attach, reconnection, or retrieval after completion.
- Persist only durable configuration required for setup: Prompt templates in extension storage, Workspace and Agent binding in local configuration, and Hermes secrets in Keychain. Do not introduce a task-history database in V0.1.
- Provide CLI commands for setup, Agent list/doctor/enable/disable, and Hermes Connection add/list/doctor. Setup and doctor are read-only and display actionable next commands. Adapter-management commands remain future ACP design and are not part of the V0.1 delivery.
- Preserve a thin Core download: prebuilt daemon, Native Host, CLI, system registration, and Keychain integration. Users do not need Rust, Node.js, npm, Bun, Python, ACP runtime, or an Agent Pack to run V0.1.
- Implement in four milestones: local bridge and CLI foundation; browser extraction and Prompt flow; Remote Hermes end-to-end; Local Claude Print Task end-to-end.
- Follow the repository comment rule: code comments are Chinese and explain business rationale, constraints, compatibility, security, concurrency, and non-obvious tradeoffs rather than restating code.

## Testing Decisions

- Tests should assert externally observable behavior at the highest practical seam. They should verify messages, files, processes, HTTP traffic, UI states, and terminal outcomes rather than private helper calls or struct layout.
- The repository currently contains only Rust and extension skeletons and no existing test prior art. Establish contract and end-to-end seams before adding narrow unit tests.
- Test the packaged Chrome extension in a real Chrome instance, loaded unpacked for development, because capture behavior, active-tab permissions, Native Messaging, popup lifetime, and page-specific extraction cannot be validated faithfully in a DOM-only test runner.
- Add browser acceptance fixtures for an ordinary article, lazy-loaded page, Twitter/X-like thread DOM, YouTube transcript state, arXiv HTML, arXiv PDF, empty extraction, oversized content, and extractor failure. Assert user-visible completeness/error behavior and normalized Captured Content.
- Test Prompt behavior through the extension UI seam: default Prompt, template selection, one-off edit, template immutability, single visible final Prompt, target-specific fields, and absence of hidden task text.
- Add protocol compatibility tests that run the Native Host as a process, feed length-prefixed Native Messaging requests, and observe responses while a fake daemon listens on a temporary Unix Socket. Cover version mismatch, malformed frame, unauthorized command, chunk ordering, chunk integrity, disconnect, and concurrent task IDs.
- Add daemon-level controller contract tests against `RemoteHermesController` and `ClaudePrintController`. Both must emit normalized lifecycle events, but capability-specific behavior should remain independently asserted.
- Test Hermes through a local fake HTTP/SSE server. Cover capability discovery, Direct URL, Bearer Token injection without logging, accepted Run, incremental SSE, non-SSE fallback, remote error, authentication error, oversized input, disconnect without cancellation, and cancel only when advertised.
- Keep live Hermes tests against a user-owned server outside the default suite. They are smoke tests for API drift and must not be required for deterministic CI.
- Test Claude using a fake executable script/binary selected by absolute path. It should emit representative stream-json, read stdin, inspect cwd and argv, spawn a child process, delay beyond timeout, return authentication errors, emit malformed JSON, and exit with success/failure codes.
- Assert at the process seam that Prompt text is delivered through stdin rather than argv, cwd equals Workspace, unrestricted Print Mode flags are present, bare/no-session-persistence/continue/resume flags are absent, and concurrent tasks receive independent UUIDs.
- Assert cancellation and timeout at the operating-system process seam: the entire process group exits, unrelated tasks survive, partial output is not final, and states are distinguished as cancelled versus timed_out versus failed.
- Test UI disconnect by closing the Native Messaging/live subscriber while fake Claude continues. Assert that the process remains alive, output is drained without unbounded memory growth, and no history/replay record is created.
- Test temporary Artifact behavior with an isolated temporary root: Markdown content and source metadata, restrictive permissions, unique Handoff directories, running-task protection, 24-hour terminal-state cleanup, crash leftovers, and missing Artifact errors.
- Test CLI commands as process-level integration tests with isolated configuration and credential abstractions. Assert read-only setup/doctor, missing Claude guidance, single/multiple executable selection, stable machine-readable statuses, Hermes Connection validation, and no implicit software installation.
- Abstract Keychain behind a narrow credential-store seam so deterministic tests can use a fake store. Add a macOS smoke test to verify real Keychain create/read/delete behavior without exposing the secret in stdout or logs.
- Add security tests that submit hostile page text, terminal control characters, path traversal names, shell metacharacters, invalid URLs, oversized fields, and unsupported commands. Assert that content cannot alter executable path, argv, Workspace, Hermes Connection, or Connector capability.
- Run Rust formatting, Clippy with workspace lints, Rust tests, TypeScript compilation, extension production build, and real-Chrome acceptance smoke tests in proportion to CI environment availability.
- Acceptance is complete when a real Chrome Handoff can capture representative content, reach a fake or controlled Hermes Run with live output, reach a fake and then real local Claude Print Task with live output, survive UI disconnect without cancelling, and stop correctly on explicit cancel or timeout.

## Out of Scope

- ACP client, Claude ACP adapter, Agent Pack installation, and Managed Session UI.
- Mid-task user messages, clarification responses, permission prompts, approval notifications, resume, attach, or replay.
- Ferry-owned response history, result database, completion notifications, badges, permanent knowledge base, or remote document inbox.
- Sandbox, container, VM, worktree isolation, per-tool permission rules, and Workspace write coordination.
- Side Panel, Web Terminal, desktop UI, or browser-independent web UI.
- Codex, OpenCode, Pi, local Hermes, and multi-Agent broadcast or result aggregation.
- Automatic Claude Code installation, authentication, upgrade, or credential import.
- Remote Hermes filesystem path selection, memory schema, indexing policy, or IM Gateway implementation.
- Chunked/multipart upload to Hermes, silent content truncation, or infinite-page completeness guarantees.
- Daemon restart recovery, durable active-task state, cross-device sync, cloud account, or public daemon endpoint.
- Linux or Windows desktop clients in the first supported release; remote Linux Hermes remains supported as a target service.
- Automatic Git branching, commit, push, pull request, issue management, worktree allocation, or project workflow orchestration.

## Further Notes

- V0.1 deliberately accepts a sharp temporary boundary: browser content is untrusted while local Claude runs with host-user permissions and no Ferry approval layer. The goal is to validate the Handoff before designing ACP permissions and sandboxing; documentation must not describe this as isolated or safe execution.
- Closing the extension UI can permanently lose visible output while the task continues. This is intentional for V0.1 and must not be “fixed” with an ad hoc history buffer; reliable reconnection belongs to the ACP/session design.
- The current codebase is an initialization skeleton. Existing launch-mode and Handoff placeholders should not constrain the accepted domain model.
- ADR 0026 defines the one-shot unrestricted Claude Print Task, ADR 0027 defines independent concurrent conversations, ADR 0028 defines live-only result delivery, and ADR 0029 applies the same delivery boundary to Remote Hermes.
- The top-level architecture and domain glossary are the authoritative vocabulary for implementation. If a code-level decision conflicts with an accepted ADR, update or supersede the ADR explicitly rather than silently diverging.
