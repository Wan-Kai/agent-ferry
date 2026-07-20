# Agent Ferry 浏览器插件 Logo 评审稿

> 状态：Exploration / Decision material  
> 日期：2026-07-20  
> 目标：为 Chrome 插件选择可在 16/32/48/128px 稳定使用的符号系统

## 设计前提

- 品牌名：`Agent Ferry`，英文大小写固定。
- 类别：本地优先的 Browser-to-Agent Handoff 开发者工具。
- 主要受众：在浏览器中阅读技术资料，并使用 Claude、Codex、OpenCode 或 Hermes 的开发者和知识工作者。
- 语气：现代、克制、可靠；技术感明确，但不冷硬。
- 主要应用：Chrome 工具栏 16/32px、扩展管理页 48px、Chrome Web Store 128px、GitHub/文档头像、单色与暗色界面。
- 明确排除：机器人、脑、AI 星芒、聊天气泡、魔法棒、具象船舶、锚和海浪。

## 方案总览

| 方向 | 架构 | 符号方法 | 16px | 单色 | 核心信号 | 主要风险 |
|---|---|---|---:|---:|---|---|
| 01 Ferry Gate | Symbol / Lockup fallback | 几何抽象 + 交接隐喻 | A | A | 可靠跨边界交接 | 初见需要一句产品说明 |
| 02 Ferry Monogram | Letterform-as-symbol | `F` 字形派生 | A− | A | 品牌名记忆 | 容易被看成通用字母 Logo |
| 03 Context Capsule | Symbol | 开发者括号 + 点阵 | C | B | 内容结构化 | 16px 元素过多 |
| 04 Converging Wake | Symbol | 抽象汇聚手势 | B | A | 提取、归一化、发送 | 类似数据管道类产品 |
| 05 Folded Handoff | Literal symbol | 页面 + 方向箭头 | B | A | Browser-to-Agent | 未来非页面入口时语义偏窄 |
| 06 Dock Network | Symbol | 节点拓扑 | C | B | daemon Hub 与目标路由 | 过像通用网络/编排工具 |

## 推荐方向：01 Ferry Gate

### 架构

浏览器插件以独立 Symbol 为主。扩展商店、README 和安装引导中使用“符号在左、`Agent Ferry` 在右”的水平 Lockup；16–128px 场景只使用符号。

### 构造

- 左侧开口边界代表浏览器与捕获入口。
- 右侧竖向 Dock 代表被明确选择的 Agent 目标。
- 中央横向 Capsule 代表完整上下文载荷，而不是普通导航箭头。
- 开口方向和载荷位置形成从左到右的自然动势，但不依赖真实箭头，因此不会退化成常见的“发送”图标。

### 色彩

| Token | 值 | 用途 |
|---|---|---|
| Ferry Blue | `#3867E8` | Chrome Web Store 与扩展管理页底板 |
| Payload Coral | `#F06B3C` | 彩色版本中的上下文载荷 |
| Ferry Ink | `#151922` | 浅色背景的单色符号与字标 |
| White | `#FFFFFF` | 蓝色底板或暗色背景反白 |

颜色不是识别前提。单色版本将三个构件统一为一个颜色，轮廓仍应成立。

### 应用规格

- 16px：只用 Symbol；至少保留 1px 外部安全区；不使用字标。
- 32/48px：可使用蓝色底板和彩色 Payload。
- 128px：使用 4px 外边距、28px 圆角底板；Chrome 会继续施加平台级视觉裁切。
- 水平 Lockup：符号高度不低于 24px；符号与字标间距等于符号宽度的 `0.24`。
- Clear space：四周至少保留中央 Payload 高度的 `0.75`。
- 单色：允许 Ferry Ink、纯黑或纯白；禁止用灰度差异区分 Payload。
- 暗色：优先使用蓝色底板彩色版；透明背景版本使用纯白，不使用降低透明度的白色。

### 动效建议

300–450ms 内让中央 Payload 从左侧开口平移到 Dock 前方，边界保持静止。动效只解释“交接”，不加入弹跳、旋转或粒子效果；遵守 `prefers-reduced-motion`。

### Signals

- 任务被可靠地从一个受控边界交给另一个受控边界。
- Ferry 传递上下文，但不扮演 Agent 本身。
- 结构简洁，适合开发者工具和本地基础设施。

### Rejects

- 不是对话产品、AI 助手人格或通用聊天入口。
- 不是云同步、网络代理或物流运输品牌。
- 不暗示 Ferry 自己完成推理或拥有宿主 Agent。

## 次选方向

### 02 Ferry Monogram

- 优势：`F` 在 1–2 秒观看中更容易与品牌名建立联系；适合社交头像和未来桌面 App。
- 约束：中横线与独立 Payload 在 16px 会形成拥挤负形，需要专用像素级小尺寸版本。
- 适用判断：如果团队更看重品牌名记忆，而不是首次看到图标时理解产品动作，可进入下一轮。

### 05 Folded Handoff

- 优势：网页内容被交出的动作最直观，扩展商店页面无需大量解释。
- 约束：折角在 16px 消失，页面轮廓和箭头也更接近现成图标库语言。
- 适用判断：如果产品长期只定位浏览器入口，可进入下一轮；如果 Ferry 未来包含更多 Connector，不建议作为主品牌符号。

## 字标建议

- 字体注册：Humanist / Neo-grotesque Sans。
- 评审阶段使用系统 `SF Pro / Inter` 风格，正式生产应选择可再分发字体并把字标转换为轮廓。
- 建议字重：`Agent` 550，`Ferry` 700；避免全小写和过度几何的 startup 字标。
- 浏览器插件图标本身不包含 `AF`、完整品牌名或任何小字号文字。

## 文件

- `07-ferry-gate-extension-icon.svg`：推荐彩色插件图标。
- `07-ferry-gate-extension-icon-mono.svg`：单色源文件。
- `browser-extension-review-board.svg`：三条入围方向的小尺寸与工具栏对比。
- `showcase.html`：六条初始方向的交互式缩放、颜色与明暗背景评审。

本轮是方向选择材料，不会直接替换 `extension/public` 或修改 Chrome manifest。选择主方向后，再生成最终的 16/32/48/128px PNG、反白资产和正式字标 Lockup。
