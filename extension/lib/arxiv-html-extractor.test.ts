import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { describe, expect, it } from "vitest";
import { extractArxivHtml, isArxivHtmlUrl } from "./arxiv-html-extractor";

describe("arXiv HTML extractor", () => {
  it("extracts the checked-in LaTeXML fixture without the navigation shell", () => {
    const fixture = readFileSync(new URL("../../test-fixtures/arxiv-html.html", import.meta.url), "utf8");
    const { document } = parseHTML(fixture);
    const result = extractArxivHtml(
      document as unknown as Document,
      "https://arxiv.org/html/2607.12345v2",
      (paperHtml) => {
        expect(paperHtml).toContain("1 Introduction");
        expect(paperHtml).toContain("E = mc^2");
        expect(paperHtml).not.toContain("arXiv navigation shell");
        expect(paperHtml).not.toContain("Report issue");
        return "## 1 Introduction\n\nBody with $E = mc^2$ and citation [1]. This paper explains how a semantic document extractor preserves provenance, readable structure, formulas, references, and enough complete prose for a downstream research agent to analyze the source reliably without mixing in the surrounding navigation shell.\n\n### 1.1 Method\n\nMethod details remain available as normalized Markdown for careful analysis and later citation.\n\n## References\n\n[1] Example Author. Reliable document extraction. 2026.";
      },
    );

    expect(result.extractor).toBe("arxiv-html");
    expect(result.title).toBe("A Lightweight Ferry for Browser-to-Agent Research");
    expect(result.author).toBe("Alice Example, Bob Researcher");
    expect(result.published).toBe("2026-07-15");
    expect(result.markdown).toContain("arXiv ID：2607.12345v2");
    expect(result.markdown).toContain("## 摘要");
    expect(result.markdown).toContain("### 1 Introduction");
    expect(result.markdown).toContain("#### 1.1 Method");
    expect(result.markdown).toContain("E = mc^2");
    expect(result.markdown).toContain("References");
    expect(result.markdown).toContain("LaTeXML 转换异常");
    expect(result.markdown).not.toContain("arXiv navigation shell");
    expect(result.markdown).not.toContain("Report issue");
  });

  it("accepts versioned modern and legacy identifiers only on arXiv HTML routes", () => {
    expect(isArxivHtmlUrl("https://arxiv.org/html/2402.08954v1")).toBe(true);
    expect(isArxivHtmlUrl("https://www.arxiv.org/html/hep-th/9901001v2")).toBe(true);
    expect(isArxivHtmlUrl("https://arxiv.org/abs/2402.08954")).toBe(false);
    expect(isArxivHtmlUrl("https://example.com/html/2402.08954")).toBe(false);
  });

  it("rejects an arXiv shell without a semantic paper body", () => {
    const { document } = parseHTML('<main class="ltx_document"><h1 class="ltx_title_document">Loading</h1><section class="ltx_bibliography">References only</section></main>');
    expect(() => extractArxivHtml(document as unknown as Document, "https://arxiv.org/html/2402.08954"))
      .toThrow("尚未加载出论文主体");
  });
});
