# Chrome Web Store 发布

> 状态：Current
> 事实来源：扩展 manifest、发行脚本、隐私政策与 Chrome Web Store 官方规则
> 范围：Chrome 扩展身份、商店字段、审核材料和发布操作；macOS Core 发行见 `release.md`

## 发布边界

Agent Ferry 的单一用途是：**把用户当前查看的网页内容和可见任务指令交给用户选择的本地或远程 AI Agent。**

历史、Prompt 模板、目标配置和连接状态都直接服务于这一次交接，不应在商店文案中包装成彼此无关的功能。扩展依赖本机 Agent Ferry Core；商店描述和首次使用界面必须在安装前后都说明这一点。

## 固定身份

首次正式打包前：

1. 使用不含 `key` 的开发 ZIP 在 Chrome Developer Dashboard 创建草稿 Item；
2. 在 Package 页面复制 32 位 Item ID 和 public key；
3. 以 `release/chrome-extension-identity.example.json` 为模板创建不提交私钥的 `release/chrome-extension-identity.json`；
4. 运行 `scripts/extension-identity.mjs` 验证 public key 推导出的 ID 与 Item ID 一致；
5. 使用 `scripts/package-chrome-extension` 生成正式 ZIP。

正式 ZIP 的 manifest `key`、Homebrew Formula 的 extension ID 和 Native Host `allowed_origins`
必须完全一致。普通用户不需要查找或输入扩展 ID。

## 商店文案

### 名称

`Agent Ferry`

### 摘要

英文 manifest 摘要（不超过 132 个字符）：

`Send the current web page and your visible prompt to a local AI Agent or your own remote Hermes.`

中文商店摘要：

`把当前网页和可见任务指令交给本地 AI Agent 或你自己的远程 Hermes。`

### 详细描述

```text
Agent Ferry hands the page you are reading to the AI Agent you choose.

Review the current page, select an Agent and where it should run, edit the complete task instruction, and start the handoff. Follow running and completed work from local task history.

• Extract the current article, X thread, or supported arXiv page only after you click Start
• Send to local Claude Code, Codex, or OpenCode in a selected Workspace
• Send to a Hermes server that you configure
• Keep reusable Prompt templates and bounded task history locally
• Use temporary active-tab access instead of permanent access to every website

Agent Ferry requires the lightweight Agent Ferry Core for macOS. It does not install or replace Claude Code, Codex, OpenCode, or Hermes. You remain responsible for the selected Agent and its provider configuration.
```

主分类选择 `Productivity`。当前扩展界面以中文为主，首个商品详情选择简体中文，并使用 `release/chrome-store/listing-zh-CN.txt` 中的文案；正式公开前再补充英文商店本地化，避免非中文 Chrome 用户只看到无法理解的列表页。

## 权限说明

Dashboard 中使用以下审核说明：

| 权限 | 审核说明 |
| --- | --- |
| `activeTab` | 在用户点击开始后，临时读取当前活动标签页；不读取其他标签页，也不持久访问全部网站。 |
| `scripting` | 把随扩展打包、可审核的页面提取脚本注入当前活动标签页。没有下载或执行远程代码。 |
| `nativeMessaging` | 把用户确认的正文和 Prompt 交给同一台 Mac 上的 Agent Ferry Core，并读取任务状态。 |
| `storage` | 在 Chrome 本地存储 Prompt 模板、Agent/位置选择和关注的任务编号。 |

发行 manifest 不得包含 `host_permissions`。增加权限必须先证明当前功能确实需要，并同步修改 UI 披露、隐私政策和 Dashboard 声明。

## 隐私 Dashboard

必须声明扩展处理：

- `Website content`：当前页面正文和页面元数据；
- `Web history`：当前活动页 URL、标题和站点；
- `User-generated content`：用户编写的 Prompt 和 Prompt 模板。

扩展不直接读取 Hermes token 或宿主 Agent 登录凭据，因此不要把 `Authentication information` 勾选成扩展收集的数据。Native Core 使用的 Hermes secret 仍需在隐私政策中如实解释。

用途只选择提供 Agent Ferry 面向用户的单一功能。完成以下认证：

- 不出售数据；
- 不把数据用于与单一用途无关的用途；
- 不把数据用于信用判断或借贷；
- 不把数据用于个性化、重定向或兴趣广告；
- 不允许人工读取，除非用户针对支持请求明确同意或法律/安全例外要求。

隐私政策 URL 使用：

`https://github.com/Wan-Kai/agent-ferry/blob/main/PRIVACY.md`

提交前必须先把同一版本的 `PRIVACY.md` 推送到公开仓库主分支，并从未登录浏览器验证可访问。Dashboard、商店文案、产品 UI 和代码行为不得互相矛盾。

## 图像和支持地址

需要准备并上传：

- 商店图标：`extension/public/icons/icon-128.png`；
- 至少一张、最多五张 `1280x800` 或 `640x400` 的真实产品截图；
- `440x280` PNG/JPEG 小型宣传图；
- 可选 `1400x560` marquee 图。

截图必须来自与提交 ZIP 相同版本的真实 Chrome，不放大或伪造未实现能力。建议顺序：发送页、执行中详情、已完成详情、任务档案、设置页。宣传图突出品牌和“网页 → Agent”关系，不直接把 popup 截图塞进宣传图。

- Homepage：`https://github.com/Wan-Kai/agent-ferry`
- Support：`https://github.com/Wan-Kai/agent-ferry/issues`

## 提交顺序

1. 在干净 commit 上运行 `./scripts/verify`；
2. 生成正式 Chrome ZIP，并记录 ZIP SHA256、版本、commit 和固定 Item ID；
3. 在真实 Chrome 加载同一发行目录，按真实环境 Runbook 完成捕获、Native Messaging、本地 Agent、远程 Hermes、历史和删除验证；
4. 确认隐私政策已公开，逐项复核 Dashboard 隐私声明；
5. 上传 ZIP、图标、截图和宣传图；
6. 先选择 Private trusted testers 完成一次商店安装验收，再切换 Public；所有可见性都需要遵守相同审核规则；
7. 提交审核但关闭自动公开发布，审核通过后再人工发布；
8. 发布后从全新 Chrome profile 安装，并重复 Core 安装和端到端验收。

## 更新与回退

- 商店每次上传必须提高 manifest 版本，并保留对应 Core release；
- 如果新扩展需要新 Core 协议，先发布向后兼容的 Core，再发布扩展；
- 若扩展回归，上传提高 patch 版本的修复包，不能尝试复用旧版本号；
- 若 Core 回归，暂停 GitHub `latest` 指向并发布修复版；已安装用户可使用固定版本 manifest 回退；
- 永久 Item ID 和 manifest public key 不随版本更换。

## 官方依据

- Chrome Web Store Program Policies：<https://developer.chrome.com/docs/webstore/program-policies/policies>
- User Data FAQ：<https://developer.chrome.com/docs/webstore/program-policies/user-data-faq>
- Dashboard privacy fields：<https://developer.chrome.com/docs/webstore/cws-dashboard-privacy>
- Store listing guidance：<https://developer.chrome.com/docs/webstore/best-listing>
- Distribution settings：<https://developer.chrome.com/docs/webstore/cws-dashboard-distribution>
