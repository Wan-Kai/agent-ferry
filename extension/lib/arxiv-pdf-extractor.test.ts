import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { prepareHandoffTransfer } from "./handoff-transfer";
import { extractArxivPdf, isArxivPdfUrl, type ArxivPdfProgress } from "./arxiv-pdf-extractor";

function fixtureFetcher(name: string, init?: ResponseInit): typeof fetch {
  const bytes = readFileSync(new URL(`../../test-fixtures/${name}`, import.meta.url));
  return (async () => new Response(bytes, {
    status: 200,
    headers: { "content-type": "application/pdf", "content-length": String(bytes.byteLength) },
    ...init,
  })) as unknown as typeof fetch;
}

describe("arXiv PDF extractor", () => {
  it("extracts metadata, pages and recognizable sections from the checked-in PDF", async () => {
    const progress: ArxivPdfProgress[] = [];
    const result = await extractArxivPdf(
      "https://arxiv.org/pdf/2607.12345v2.pdf",
      fixtureFetcher("arxiv-paper.pdf"),
      (event) => progress.push(event),
    );

    expect(result.extractor).toBe("arxiv-pdf");
    expect(result.title).toBe("A Lightweight PDF Ferry for Research Agents");
    expect(result.author).toBe("Alice Example and Bob Researcher");
    expect(result.markdown).toContain("arXiv ID：2607.12345v2");
    expect(result.markdown).toContain("PDF 页数：2");
    expect(result.markdown).toContain("## 第 1 页");
    expect(result.markdown).toContain("### 1 Introduction");
    expect(result.markdown).toContain("### 2 Method");
    expect(result.markdown).toContain("### References");
    expect(result.markdown).toContain("完整性提示");
    expect(result.word_count).toBeGreaterThan(40);
    expect(progress).toContainEqual(expect.objectContaining({ stage: "downloading", loaded_bytes: 0 }));
    expect(progress).toContainEqual(expect.objectContaining({ stage: "opening" }));
    expect(progress).toContainEqual({ stage: "extracting", completed_pages: 0, total_pages: 2 });
    expect(progress.at(-1)).toEqual({ stage: "extracting", completed_pages: 2, total_pages: 2 });
  });

  it("recognizes modern and legacy PDF routes without accepting lookalike hosts", () => {
    expect(isArxivPdfUrl("https://arxiv.org/pdf/2402.08954")).toBe(true);
    expect(isArxivPdfUrl("https://www.arxiv.org/pdf/2402.08954V2.pdf?download=1")).toBe(true);
    expect(isArxivPdfUrl("https://arxiv.org/pdf/hep-th/9901001v1.pdf")).toBe(true);
    expect(isArxivPdfUrl("https://arxiv.org/html/2402.08954")).toBe(false);
    expect(isArxivPdfUrl("ftp://arxiv.org/pdf/2402.08954")).toBe(false);
    expect(isArxivPdfUrl("https://arxiv.org/PDF/2402.08954.pdf")).toBe(false);
    expect(isArxivPdfUrl("https://arxiv.org/pdf/2402.089%35%34.pdf")).toBe(false);
    expect(isArxivPdfUrl("https://example.com/pdf/2402.08954.pdf")).toBe(false);
  });

  it("reports password-protected PDFs explicitly", async () => {
    await expect(extractArxivPdf(
      "https://arxiv.org/pdf/2607.12345",
      fixtureFetcher("arxiv-protected.pdf"),
    )).rejects.toThrow("受密码保护");
  });

  it("reports damaged PDFs explicitly", async () => {
    await expect(extractArxivPdf(
      "https://arxiv.org/pdf/2607.12345",
      fixtureFetcher("arxiv-corrupt.pdf"),
    )).rejects.toThrow("损坏或格式无法解析");
  });

  it("reports PDFs without a text layer instead of attempting OCR", async () => {
    await expect(extractArxivPdf(
      "https://arxiv.org/pdf/2607.12345",
      fixtureFetcher("arxiv-no-text.pdf"),
    )).rejects.toThrow("没有可提取的文本层");
  });

  it("distinguishes HTTP, non-PDF and oversized download failures", async () => {
    const httpFailure = (async () => new Response("missing", { status: 404 })) as unknown as typeof fetch;
    await expect(extractArxivPdf("https://arxiv.org/pdf/2607.12345", httpFailure)).rejects.toThrow("HTTP 404");

    const html = (async () => new Response("<!doctype html>not a PDF")) as unknown as typeof fetch;
    await expect(extractArxivPdf("https://arxiv.org/pdf/2607.12345", html)).rejects.toThrow("不是有效的 PDF");

    const oversized = (async () => new Response("%PDF-", {
      headers: { "content-length": String(65 * 1024 * 1024) },
    })) as unknown as typeof fetch;
    await expect(extractArxivPdf("https://arxiv.org/pdf/2607.12345", oversized)).rejects.toThrow("64 MiB");
  });

  it("reports a response stream that breaks after download begins", async () => {
    const interrupted = (async () => new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("%PDF-1.7\n"));
        controller.error(new Error("connection reset"));
      },
    }))) as unknown as typeof fetch;
    await expect(extractArxivPdf("https://arxiv.org/pdf/2607.12345", interrupted)).rejects.toThrow("PDF 下载中断");
  });

  it("sends large extracted Markdown through the existing lossless framing", async () => {
    const result = await extractArxivPdf(
      "https://arxiv.org/pdf/2607.12345",
      fixtureFetcher("arxiv-paper.pdf"),
    );
    const markdown = `${result.markdown}\n${result.markdown.repeat(90)}`;
    const transfer = await prepareHandoffTransfer(markdown);
    expect(transfer.chunks.length).toBeGreaterThan(1);
    expect(transfer.chunks.join("")).toBe(markdown);
  });
});
