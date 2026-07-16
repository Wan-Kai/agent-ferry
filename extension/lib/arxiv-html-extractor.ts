import Defuddle from "defuddle";

export type ArxivHtmlCapture = {
  url: string;
  title: string;
  author: string | null;
  published: string | null;
  site: "arXiv";
  extractor: "arxiv-html";
  markdown: string;
  word_count: number;
};

const ARXIV_HOSTS = new Set(["arxiv.org", "www.arxiv.org"]);
const MODERN_IDENTIFIER = /^\d{4}\.\d{4,5}(?:v\d+)?$/i;
const LEGACY_IDENTIFIER = /^[a-z-]+(?:\.[a-z]{2})?\/\d{7}(?:v\d+)?$/i;

function arxivIdentifier(value: string): string | null {
  try {
    const url = new URL(value);
    if (!ARXIV_HOSTS.has(url.hostname.toLowerCase())) return null;
    const encoded = url.pathname.match(/^\/html\/(.+?)\/?$/)?.[1];
    if (!encoded) return null;
    const identifier = decodeURIComponent(encoded);
    return MODERN_IDENTIFIER.test(identifier) || LEGACY_IDENTIFIER.test(identifier) ? identifier : null;
  } catch {
    return null;
  }
}

export function isArxivHtmlUrl(value: string): boolean {
  return arxivIdentifier(value) !== null;
}

function normalizedText(value: string | null | undefined): string {
  return (value ?? "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim();
}

function countWords(value: string): number {
  const cjk = value.match(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g)?.length ?? 0;
  const words = value.replace(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g, " ").match(/[\p{L}\p{N}_]+/gu)?.length ?? 0;
  return cjk + words;
}

function metaContent(document: Document, names: readonly string[]): string | null {
  for (const name of names) {
    const value = document.querySelector(`meta[name="${name}"], meta[property="${name}"]`)?.getAttribute("content");
    if (normalizedText(value)) return normalizedText(value);
  }
  return null;
}

function formulaSource(element: Element): string | null {
  const direct = element.getAttribute("alttext") ?? element.querySelector("math[alttext]")?.getAttribute("alttext");
  if (normalizedText(direct)) return normalizedText(direct);
  const annotation = Array.from(element.querySelectorAll("annotation")).find((node) =>
    /(?:x-)?tex/i.test(node.getAttribute("encoding") ?? ""),
  );
  return normalizedText(annotation?.textContent) || null;
}

function abstractMarkdown(abstract: Element | null): string {
  if (!abstract) return "（页面未提供可识别的摘要。）";
  const clone = abstract.cloneNode(true) as Element;
  clone.querySelectorAll(".ltx_title_abstract").forEach((node) => node.remove());
  clone.querySelectorAll("math, .ltx_Math").forEach((node) => {
    const source = formulaSource(node);
    if (source) node.replaceWith(clone.ownerDocument.createTextNode(`$${source}$`));
  });
  return Array.from(clone.querySelectorAll(".ltx_p"))
    .map((paragraph) => normalizedText(paragraph.textContent))
    .filter(Boolean)
    .join("\n\n") || normalizedText(clone.textContent) || "（页面未提供可识别的摘要。）";
}

/**
 * 论文正文被放在“论文正文”二级标题之下，因此把 Defuddle 产生的最高级标题下移到三级。
 * 只处理围栏代码块之外的 Markdown，避免把论文中的代码样例误当成标题改写。
 */
function nestHeadings(markdown: string): string {
  const lines = markdown.split("\n");
  let fenced = false;
  let minimum = 7;
  for (const line of lines) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    if (!fenced) minimum = Math.min(minimum, line.match(/^(#{1,6})\s+/)?.[1].length ?? 7);
  }
  if (minimum > 6) return markdown;
  const shift = Math.max(0, 3 - minimum);
  fenced = false;
  return lines.map((line) => {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    if (fenced || shift === 0) return line;
    return line.replace(/^(#{1,6})(\s+)/, (_, hashes: string, whitespace: string) => `${"#".repeat(Math.min(6, hashes.length + shift))}${whitespace}`);
  }).join("\n");
}

function bodyMarkdown(document: Document, root: Element, pageUrl: string, convert?: (html: string, url: string) => string): string {
  // Defuddle 已针对 arXiv LaTeXML 实现公式、引用和参考文献转换；专用层只裁掉页面外壳与重复的题录信息。
  const snapshot = document.cloneNode(true) as Document;
  const paper = root.cloneNode(true) as Element;
  paper.querySelectorAll(".ltx_title_document, .ltx_authors, .ltx_abstract").forEach((node) => node.remove());
  snapshot.body.replaceChildren(paper);
  const parsed = new Defuddle(snapshot, { markdown: convert === undefined, useAsync: false, url: pageUrl }).parse();
  const markdown = convert ? convert(parsed.content ?? "", pageUrl) : (parsed.contentMarkdown ?? parsed.content ?? "");
  return nestHeadings(markdown.trim());
}

/**
 * 背景与目的：arXiv HTML 是 LaTeXML 语义文档，通用正文选择有时会混入导航、实验状态和反馈外壳。
 * 专用提取器先锁定 `.ltx_document`，再复用 Defuddle 已有的 LaTeXML 转换，避免维护第二套公式与引用解析器。
 *
 * 边界情况与测试要点：
 * - 页面缺摘要、章节或公式源时仍交付可读正文，但必须显式提示完整性风险。
 * - URL 中的版本号属于来源标识，不能静默丢弃。
 */
export function extractArxivHtml(
  document: Document,
  pageUrl: string,
  convert?: (html: string, url: string) => string,
): ArxivHtmlCapture {
  const identifier = arxivIdentifier(pageUrl);
  if (!identifier) throw new Error("当前页面不是可提取的 arXiv HTML 论文");
  const root = document.querySelector("article.ltx_document, .ltx_document");
  const title = normalizedText(root?.querySelector(".ltx_title_document")?.textContent)
    || metaContent(document, ["citation_title", "og:title"])
    || "";
  if (!root || !title || !root.querySelector(".ltx_section, .ltx_abstract")) {
    throw new Error("arXiv HTML 尚未加载出论文主体，请等待页面完成加载后重试");
  }

  const authors = Array.from(root.querySelectorAll(".ltx_authors .ltx_personname"))
    .map((node) => normalizedText(node.textContent))
    .filter((author, index, all) => author && all.indexOf(author) === index);
  if (authors.length === 0) {
    document.querySelectorAll('meta[name="citation_author"]').forEach((node) => {
      const author = normalizedText(node.getAttribute("content"));
      if (author && !authors.includes(author)) authors.push(author);
    });
  }
  const abstract = root.querySelector(".ltx_abstract");
  const body = bodyMarkdown(document, root, pageUrl, convert);
  if (body.length < 200 || countWords(body) < 40) {
    throw new Error("arXiv 论文正文过短，可能尚未加载完整，请稍后重试");
  }

  const warnings: string[] = [];
  if (!abstract) warnings.push("页面未提供可识别的摘要");
  if (!root.querySelector(".ltx_section")) warnings.push("页面未提供可识别的章节结构");
  const formulas = Array.from(root.querySelectorAll("math, .ltx_Math"));
  const missingFormulaSources = formulas.filter((formula) => !formulaSource(formula)).length;
  if (missingFormulaSources > 0) warnings.push(`${missingFormulaSources} 个公式缺少可恢复的 TeX 源`);
  if (root.querySelector(".ltx_ERROR, .ltx_problem, [data-latexml-error]")) warnings.push("arXiv 页面报告了 LaTeXML 转换异常");
  if (root.querySelector(".ltx_bibliography") && !/(参考文献|references|bibliography)/i.test(body)) {
    warnings.push("页面含参考文献，但 Markdown 中未识别出参考文献标题");
  }

  const canonicalUrl = `https://arxiv.org/html/${identifier}`;
  const sections = [
    `# ${title}`,
    "",
    `- arXiv ID：${identifier}`,
    `- 论文 URL：${canonicalUrl}`,
    `- 作者：${authors.join("；") || "未知"}`,
    "- 提取来源：arXiv HTML（LaTeXML）",
  ];
  if (warnings.length > 0) sections.push("", `> 完整性提示：${warnings.join("；")}。`);
  sections.push("", "## 摘要", "", abstractMarkdown(abstract), "", "## 论文正文", "", body);
  const markdown = sections.join("\n").trim();
  return {
    url: pageUrl,
    title,
    author: authors.join(", ") || null,
    published: metaContent(document, ["citation_date", "citation_publication_date", "article:published_time"]),
    site: "arXiv",
    extractor: "arxiv-html",
    markdown,
    word_count: countWords(markdown),
  };
}
