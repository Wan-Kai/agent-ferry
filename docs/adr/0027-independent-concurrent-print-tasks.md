# ADR 0027：每次交接创建独立且可并发的 Print Task

## 状态

已接受，2026-07-15。

## 背景

同一 Workspace 的任务串行可以减少文件冲突，但会让一个长任务阻塞后续网页交接。V0.1 的目标是把每次捕获直接交给一个新的 Claude 对话，而不是建立由 Ferry 管理的 Workspace 作业队列。

## 决策

1. 每次 Handoff 都创建新的 Ferry Task 和新的 Claude Code Print 对话；
2. daemon 为任务生成唯一 UUID，并在 Claude Code 支持时通过 `--session-id` 显式绑定，绝不使用 `--continue` 或 `--resume`；
3. 同一 Workspace 的多个任务允许同时启动和运行，不设置 Workspace 锁或串行队列；
4. 不同任务的进程、stdin、stdout/stderr、deadline、取消信号、Artifact 和最终结果分别归档；
5. Ferry 不协调多个 Claude 进程对同一 Workspace 的文件读写、Git 状态或外部资源操作；
6. 浏览器必须把它们展示为独立任务，取消其中一个不能终止其他任务；
7. “独立对话”不表示 V0.1 支持用户续写；每个对话仍只有一次输入，完成后结束。

## 后果

- 后续 Handoff 不会被同一 Workspace 的长任务阻塞；
- 用户可以同时分析多个页面；
- 同 Workspace 并发写入可能产生覆盖、Git 冲突或外部副作用竞争，这是 V0.1 `unrestricted_host` 接受的边界；
- 后续 sandbox、worktree 或 ACP 模式可以提供更强隔离，但不能改变已有任务的运行语义。
