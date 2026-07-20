# ADR 0004：使用 Defuddle 的混合内容提取架构

## 状态

已接受，2026-07-14。

## 背景

用户希望尽量获得网页全文，同时明确首要特殊来源包括 Twitter/X、YouTube、arXiv PDF 和 arXiv HTML。不同站点的 DOM、懒加载和内容接口差异很大，单一通用算法不能稳定覆盖这些来源。

Obsidian Web Clipper 的内容提取主要建立在 Defuddle 上。Defuddle 提供浏览器端页面解析、元数据、Markdown 转换以及 Twitter/X 和 YouTube 等专用 Extractor，采用 MIT 许可证并持续维护。

## 决策

Agent Ferry 的浏览器扩展将 Defuddle 作为核心内容提取依赖，并跟随上游版本升级，不复制或 fork 整个 Obsidian Web Clipper。

内容提取采用混合策略：

1. 普通站点执行有时间、轮次和内容量上限的懒加载触发，恢复用户原滚动位置后，使用 Defuddle 提取内容；
2. Twitter/X 和 YouTube 优先使用 Defuddle 已有的专用 Extractor；
3. arXiv HTML 首先使用 Defuddle 通用解析，根据验证结果增加轻量适配；
4. arXiv PDF 使用独立的原始 PDF 获取与解析链路，不尝试读取浏览器 PDF Viewer DOM；
5. 捕获结果必须记录提取器、完整性状态和降级原因。

可以选择性参考或移植 Obsidian Web Clipper 中与浏览器内容读取、Shadow DOM 展平、变量构造有关的少量 MIT 代码，但不引入其 Vault、笔记属性、Highlight、Reader、Interpreter 或 Obsidian URI 逻辑。

## 备选方案

### 复制整个 Obsidian Web Clipper

不采用。它的大量代码服务于 Obsidian 笔记工作流，与 Agent Ferry 的工作区交接和 Agent 启动无关，会造成不必要的耦合。

### Fork Defuddle

不作为默认方案。Fork 会失去低成本获得上游站点修复的优势。只有上游接口无法满足关键安全或产品约束时，才重新评估。

### 只使用通用整页 DOM 转换

不采用。它无法可靠处理 Twitter/X、YouTube 字幕和 PDF 等首要来源。

## 后果

- 扩展构建将增加 Defuddle 及其相关许可证声明；
- Defuddle 升级需要通过固定的内容样例回归测试，不能无验证追随最新版本；
- Twitter/X 和 YouTube 的非官方接口或第三方回退必须可识别、可降级，并在涉及外部服务时遵守隐私约束；
- arXiv PDF 需要单独设计二进制内容传输、解析和 artifact 结构；
- Prompt 模板系统仍是 Agent Ferry 的任务说明系统，不复用 Obsidian 的笔记模板领域模型。
