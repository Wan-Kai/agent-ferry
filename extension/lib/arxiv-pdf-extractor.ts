import { MAX_HANDOFF_CONTENT_BYTES } from "./handoff-transfer";

export type ArxivPdfCapture = {
  url: string;
  title: string;
  author: string | null;
  published: string | null;
  site: "arXiv";
  extractor: "arxiv-pdf";
  markdown: string;
  word_count: number;
};

export type ArxivPdfProgress =
  | { stage: "downloading"; loaded_bytes: number; total_bytes: number | null }
  | { stage: "opening"; total_bytes: number }
  | { stage: "extracting"; completed_pages: number; total_pages: number };

type ProgressReporter = (progress: ArxivPdfProgress) => void;

const ARXIV_HOSTS = new Set(["arxiv.org", "www.arxiv.org"]);
const MODERN_IDENTIFIER = /^\d{4}\.\d{4,5}(?:v\d+)?$/i;
const LEGACY_IDENTIFIER = /^[a-z-]+(?:\.[a-z]{2})?\/\d{7}(?:v\d+)?$/i;
const MAX_PDF_BYTES = 64 * 1024 * 1024;

function arxivPdfIdentifier(value: string): string | null {
  try {
    const url = new URL(value);
    if (!["http:", "https:"].includes(url.protocol)) return null;
    if (!ARXIV_HOSTS.has(url.hostname.toLowerCase())) return null;
    const encoded = url.pathname.match(/^\/pdf\/(.+?)(?:\.[pP][dD][fF])?\/?$/)?.[1];
    if (!encoded) return null;
    // arXiv 标识本身只使用 ASCII；拒绝百分号编码，确保浏览器与 daemon 对来源绑定完全一致。
    const identifier = encoded;
    return MODERN_IDENTIFIER.test(identifier) || LEGACY_IDENTIFIER.test(identifier) ? identifier : null;
  } catch {
    return null;
  }
}

export function isArxivPdfUrl(value: string): boolean {
  return arxivPdfIdentifier(value) !== null;
}

function normalizedText(value: unknown): string {
  return typeof value === "string" ? value.replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim() : "";
}

function countWords(value: string): number {
  const cjk = value.match(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g)?.length ?? 0;
  const words = value.replace(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g, " ").match(/[\p{L}\p{N}_]+/gu)?.length ?? 0;
  return cjk + words;
}

function parsePdfDate(value: unknown): string | null {
  const text = normalizedText(value);
  const match = text.match(/^(?:D:)?(\d{4})(\d{2})(\d{2})/);
  return match ? `${match[1]}-${match[2]}-${match[3]}` : null;
}

function isSectionHeading(line: string): boolean {
  if (line.length < 2 || line.length > 120) return false;
  if (/^\d+(?:\.\d+)*\.?\s+[\p{L}][\p{L}\p{N}\s,:()/-]{1,100}$/u.test(line)) return true;
  return /^(abstract|introduction|background|related work|method(?:ology)?|approach|experiments?|results?|discussion|conclusion|acknowledg(?:e)?ments?|references|appendix)$/i.test(line);
}

type PdfTextItem = { str: string; hasEOL: boolean };

function pageLines(items: readonly PdfTextItem[]): string[] {
  const lines: string[] = [];
  let current = "";
  for (const item of items) {
    const text = normalizedText(item.str);
    if (text) current = current ? `${current} ${text}` : text;
    if (item.hasEOL && current) {
      lines.push(current);
      current = "";
    }
  }
  if (current) lines.push(current);
  return lines;
}

function formatPage(items: readonly PdfTextItem[], pageNumber: number): { markdown: string; lines: string[] } {
  const lines = pageLines(items);
  const sections = [`## 第 ${pageNumber} 页`];
  for (const line of lines) sections.push(isSectionHeading(line) ? `\n### ${line}\n` : line);
  return { markdown: sections.join("\n").replace(/\n{3,}/g, "\n\n").trim(), lines };
}

function classifyPdfError(error: unknown): Error {
  const name = error instanceof Error ? error.name : "";
  const message = error instanceof Error ? error.message : String(error);
  if (name === "PasswordException" || /password/i.test(message)) return new Error("PDF 受密码保护，当前版本无法读取正文");
  if (["InvalidPDFException", "FormatError", "MissingPDFException"].includes(name) || /invalid pdf|xref|format error/i.test(message)) {
    return new Error("PDF 文件已损坏或格式无法解析");
  }
  return new Error(`PDF 解析失败：${message || "未知错误"}`);
}

async function readResponseBytes(response: Response, reportProgress?: ProgressReporter): Promise<Uint8Array> {
  const announced = Number(response.headers.get("content-length") ?? "0");
  if (Number.isFinite(announced) && announced > MAX_PDF_BYTES) throw new Error("PDF 超过当前版本的 64 MiB 下载上限");
  const totalBytes = announced > 0 ? announced : null;
  reportProgress?.({ stage: "downloading", loaded_bytes: 0, total_bytes: totalBytes });
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > MAX_PDF_BYTES) throw new Error("PDF 超过当前版本的 64 MiB 下载上限");
    reportProgress?.({ stage: "downloading", loaded_bytes: bytes.byteLength, total_bytes: totalBytes });
    return bytes;
  }
  const reader = response.body.getReader();
  // 单一缓冲区避免“chunks + 合并结果”同时占用约两倍 PDF 大小；未知长度时按需倍增。
  let bytes = new Uint8Array(announced > 0 ? announced : Math.min(1024 * 1024, MAX_PDF_BYTES));
  let total = 0;
  let lastReported = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_PDF_BYTES) {
      await reader.cancel();
      throw new Error("PDF 超过当前版本的 64 MiB 下载上限");
    }
    if (total > bytes.byteLength) {
      let capacity = Math.max(1, bytes.byteLength);
      while (capacity < total) capacity = Math.min(MAX_PDF_BYTES, capacity * 2);
      const grown = new Uint8Array(capacity);
      grown.set(bytes);
      bytes = grown;
    }
    bytes.set(value, total - value.byteLength);
    // 限制 React 更新频率；小网络分片不应让大文件下载反而因进度渲染变慢。
    if (total - lastReported >= 256 * 1024 || (totalBytes !== null && total >= totalBytes)) {
      lastReported = total;
      reportProgress?.({ stage: "downloading", loaded_bytes: total, total_bytes: totalBytes });
    }
  }
  if (total !== lastReported) reportProgress?.({ stage: "downloading", loaded_bytes: total, total_bytes: totalBytes });
  return bytes.subarray(0, total);
}

type PdfProxy = Awaited<ReturnType<(typeof import("unpdf"))["getDocumentProxy"]>>;

function yieldToPopup(reportProgress?: ProgressReporter): Promise<void> {
  // PDF.js 的 serverless 构建可能在弹窗主线程解析；让出一个宏任务，确保进度先完成绘制。
  return reportProgress ? new Promise((resolve) => setTimeout(resolve, 0)) : Promise.resolve();
}

async function extractPages(pdf: PdfProxy, reportProgress?: ProgressReporter): Promise<{ markdown: string; firstPageLines: string[] }> {
  const pages: string[] = [];
  let firstPageLines: string[] = [];
  let totalBytes = 0;
  reportProgress?.({ stage: "extracting", completed_pages: 0, total_pages: pdf.numPages });
  for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber += 1) {
    await yieldToPopup(reportProgress);
    const page = await pdf.getPage(pageNumber);
    try {
      const content = await page.getTextContent();
      const items = content.items
        .filter((item): item is typeof item & { str: string; hasEOL: boolean } => "str" in item)
        .map((item) => ({ str: item.str, hasEOL: item.hasEOL }));
      const formatted = formatPage(items, pageNumber);
      if (pageNumber === 1) firstPageLines = formatted.lines;
      totalBytes += new TextEncoder().encode(formatted.markdown).byteLength + 2;
      if (totalBytes > MAX_HANDOFF_CONTENT_BYTES) {
        throw new Error("PDF 提取正文超过当前版本的 8 MiB 传输上限");
      }
      pages.push(formatted.markdown);
      reportProgress?.({ stage: "extracting", completed_pages: pageNumber, total_pages: pdf.numPages });
    } finally {
      page.cleanup();
    }
  }
  return { markdown: pages.join("\n\n"), firstPageLines };
}

/**
 * 背景与目的：Chrome 内置 PDF Viewer 不是普通网页，无法通过 content script 读取论文 DOM。
 * 因此弹窗直接获取原始 PDF，并仅在 PDF 路径上按需加载解析器；普通网页不会加载 PDF.js 模块。
 *
 * 设计理由与约束：
 * - 原始 PDF 限制为 64 MiB，防止错误 Content-Length 或恶意响应耗尽弹窗内存。
 * - 输出 Markdown 仍经过既有 8 MiB 分帧协议；任何超限都显式失败，不允许静默截断。
 * - PDF 文本层无法可靠恢复公式、图片和精确阅读顺序，因此每次交付都附带完整性提示。
 */
export async function extractArxivPdf(
  pageUrl: string,
  fetcher: typeof fetch = fetch,
  reportProgress?: ProgressReporter,
): Promise<ArxivPdfCapture> {
  const identifier = arxivPdfIdentifier(pageUrl);
  if (!identifier) throw new Error("当前页面不是可提取的 arXiv PDF 论文");
  let response: Response;
  try {
    response = await fetcher(pageUrl, { credentials: "omit", redirect: "follow" });
  } catch {
    throw new Error("arXiv PDF 下载失败，请检查网络后重试");
  }
  if (!response.ok) throw new Error(`arXiv PDF 下载失败（HTTP ${response.status}）`);
  let bytes: Uint8Array;
  try {
    bytes = await readResponseBytes(response, reportProgress);
  } catch (error) {
    if (error instanceof Error && error.message.includes("64 MiB")) throw error;
    throw new Error("arXiv PDF 下载中断，请检查网络后重试");
  }
  if (bytes.byteLength < 5 || new TextDecoder("ascii").decode(bytes.subarray(0, 5)) !== "%PDF-") {
    throw new Error("下载结果不是有效的 PDF 文件");
  }

  let pdf: PdfProxy | null = null;
  try {
    reportProgress?.({ stage: "opening", total_bytes: bytes.byteLength });
    await yieldToPopup(reportProgress);
    const { getDocumentProxy } = await import("unpdf");
    pdf = await getDocumentProxy(bytes);
    const [{ info }, formatted] = await Promise.all([pdf.getMetadata(), extractPages(pdf, reportProgress)]);
    if (formatted.markdown.length < 200 || countWords(formatted.markdown) < 40) {
      throw new Error("PDF 没有可提取的文本层，当前版本不执行 OCR");
    }
    const record = info as Record<string, unknown>;
    const title = normalizedText(record.Title) || formatted.firstPageLines[0] || `arXiv ${identifier}`;
    const titleIndex = formatted.firstPageLines.findIndex((line) => line === title);
    const authorCandidate = titleIndex >= 0 ? formatted.firstPageLines[titleIndex + 1] ?? "" : "";
    // arXiv PDF 经常不写 Author metadata；只在标题紧邻行满足保守条件时补回，避免把摘要误报成作者。
    const inferredAuthor = authorCandidate.length <= 300
      && !isSectionHeading(authorCandidate)
      && !/^(?:arxiv:|abstract\b)/i.test(authorCandidate)
      ? authorCandidate
      : "";
    const author = normalizedText(record.Author) || inferredAuthor || null;
    const published = parsePdfDate(record.CreationDate ?? record.ModDate);
    const markdown = [
      `# ${title}`,
      "",
      `- arXiv ID：${identifier}`,
      `- 论文 URL：https://arxiv.org/pdf/${identifier}`,
      `- 作者：${author ?? "未知"}`,
      `- PDF 页数：${pdf.numPages}`,
      "- 提取来源：arXiv PDF 文本层",
      "",
      "> 完整性提示：PDF 文本层可能无法保留精确阅读顺序、公式结构、图片内容和版面关系；当前版本不执行 OCR。",
      "",
      formatted.markdown,
    ].join("\n").trim();
    return { url: pageUrl, title, author, published, site: "arXiv", extractor: "arxiv-pdf", markdown, word_count: countWords(markdown) };
  } catch (error) {
    if (error instanceof Error && (
      error.message.includes("没有可提取的文本层")
      || error.message.includes("64 MiB")
      || error.message.includes("8 MiB")
    )) throw error;
    throw classifyPdfError(error);
  } finally {
    await pdf?.destroy();
  }
}
