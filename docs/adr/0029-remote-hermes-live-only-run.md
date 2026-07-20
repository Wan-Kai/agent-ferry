# ADR 0029：云端 Hermes 首版采用实时、非持久化 Run

## 状态

Superseded，2026-07-19。Hermes 所有权边界仍有效；Ferry 本地任务历史由 ADR 0030 统一定义。

## 背景

云端 Hermes 已通过 IM Gateway 管理自己的 profile、文件、记忆和会话。若 Ferry 再保存 Runs API 结果，会重复建设结果历史和召回体系。V0.1 只需把浏览器内容可靠提交，并在界面仍连接时转发实时结果。

## 决策

1. V0.1 将有效 Prompt、来源和完整 Markdown 提交给 Hermes Runs API；
2. Hermes 声明 SSE 能力时，daemon 将状态和文本增量实时转发给当前浏览器界面；否则只展示提交与终态能力实际支持的状态；
3. Ferry 不保存 Hermes 回复正文、最终答案或可重放事件；
4. 浏览器界面关闭不取消远程 Run；daemon 可以结束本地 SSE 订阅，远程任务继续由 Hermes 管理；
5. Ferry 不提供重新 attach、遗漏补发或完成后取回；用户后续通过 IM 和 Hermes 自己的记忆/文件能力继续使用结果；
6. Hermes 是否保存捕获内容、保存到哪里以及如何索引，继续由 Hermes 自己判断；
7. 后续 ACP/可靠会话阶段再统一设计重连游标、事件存储、通知和历史 UI。

## 后果

- 本地 Claude 与云端 Hermes 在 V0.1 都使用实时、非持久化结果 UI；
- Ferry 不成为第二套 Hermes 记忆或会话数据库；
- 关闭浏览器界面后无法从 Ferry 取回遗漏的 Hermes 输出；
- Hermes Run 的远程生命周期不依赖 Native Host 或扩展页面是否仍连接。
