# Chrome Extension 模块地图

## 职责

在真实 Chrome 中捕获当前页面，展示最终 Prompt 和可用目标，通过 Native Messaging 提交任务，并呈现实时状态与本地历史。

## 关键成员

- `entrypoints/background.ts`：Native Messaging 请求和扩展后台协调。
- `entrypoints/extract-page.ts`：只在用户点击后通过 `activeTab` 注入的页面捕获入口。
- `entrypoints/popup/main.tsx`：发送、历史、详情与设置 UI。
- `lib/`：提取器、传输分块、Prompt、目标选择和历史视图纯逻辑。
- `test-fixtures/`：仓库根目录的页面与 PDF 测试样本。

## 依赖关系

Extension 只依赖浏览器 API 和 Protocol JSON 契约，不读取 daemon 配置、Agent 凭据或宿主文件。所有管理能力经授权后的 daemon 命令完成。

## 不变量

- 用户任务意图只来自可见的最终 Prompt，不追加隐藏任务指令。
- 页面内容不可信，不能控制 target、Workspace、命令或连接参数。
- 大正文使用有界分块传输，失败时明确告知用户，不能静默只发送 URL。
- DOM 测试不能替代 Popup 生命周期、activeTab、Native Messaging 和权限的真实 Chrome 验收。
- 页面捕获必须使用 `activeTab` 的临时授权，不得为按需读取当前页申请持久全站 host permission。
- UI 关闭与任务取消是两个独立动作。

## 变更影响

- 修改消息类型时同步检查 Protocol 和 daemon。
- 修改提取器时覆盖普通页面、懒加载、X、arXiv HTML/PDF、空内容和超限场景。
- 修改任务历史时同步检查 daemon 持久化、轮询、删除限制和隐私展示。

## 验证

```bash
npm test
npm run compile
npm run build
```

涉及浏览器行为时，继续按 `docs/runbooks/real-environment-acceptance.md` 使用真实 Chrome 验收。

## 关联文档

- [PROTOCOL] `docs/architecture/overview.md`
- `docs/runbooks/real-environment-acceptance.md`
