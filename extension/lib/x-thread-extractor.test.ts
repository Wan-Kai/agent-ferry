import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { describe, expect, it } from "vitest";
import { extractXThread, extractXThreadSnapshots, isXStatusUrl } from "./x-thread-extractor";

function tweet(handle: string, id: string, text: string, name = handle): string {
  return `<div data-testid="cellInnerDiv"><article data-testid="tweet">
    <div data-testid="User-Name"><a href="/${handle.slice(1)}">${name}</a><a href="/${handle.slice(1)}">${handle}</a></div>
    <a href="/${handle.slice(1)}/status/${id}"><time datetime="2026-07-16T01:02:03.000Z"></time></a>
    <div data-testid="tweetText">${text}</div>
  </article></div>`;
}

describe("X thread extractor", () => {
  it("extracts the checked-in DOM fixture", () => {
    const fixture = readFileSync(new URL("../../test-fixtures/x-thread.html", import.meta.url), "utf8");
    const { document } = parseHTML(fixture);
    const result = extractXThread(document as unknown as Document, "https://x.com/agentferry/status/100");
    expect(result.markdown).toContain("Agent Ferry @agentferry");
    expect(result.markdown).toContain("https://x.com/reader/status/102");
    expect(result.markdown).toContain("## 可见回复");
  });

  it("extracts the root, self-thread and visible reply metadata with hierarchy", () => {
    const { document } = parseHTML(`<main aria-label="Timeline: Conversation">
      ${tweet("@alice", "100", "主帖正文", "Alice")}
      ${tweet("@alice", "101", "续帖正文", "Alice")}
      ${tweet("@bob", "102", "第一层回复", "Bob")}
      ${tweet("@carol", "103", "连续可见的嵌套回复", "Carol")}
    </main>`);
    const result = extractXThread(document as unknown as Document, "https://x.com/alice/status/101");
    expect(result.extractor).toBe("x-thread");
    expect(result.author).toBe("Alice @alice");
    expect(result.published).toBe("2026-07-16T01:02:03.000Z");
    expect(result.markdown).toContain("### 主帖");
    expect(result.markdown).toContain("### 同作者续帖 1（当前页面）");
    expect(result.markdown).toContain("- 链接：https://x.com/alice/status/100");
    expect(result.markdown).toContain("- 链接：https://x.com/bob/status/102");
    expect(result.markdown).toContain("> ### 回复 2");
    expect(result.markdown).not.toContain("> > ### 回复 2");
  });

  it("merges posts exposed at different virtual-list scroll positions", () => {
    const initial = parseHTML(`<main aria-label="Timeline: Conversation">${tweet("@alice", "100", "主帖")}${tweet("@bob", "102", "初始可见回复")}</main>`).document;
    const final = parseHTML(`<main aria-label="Timeline: Conversation">${tweet("@alice", "100", "主帖")}${tweet("@alice", "101", "后续快照补出的同作者续帖")}${tweet("@bob", "102", "初始可见回复")}${tweet("@carol", "103", "滚动后新增回复")}</main>`).document;
    const result = extractXThreadSnapshots(
      [initial as unknown as Document, final as unknown as Document],
      "https://x.com/alice/status/100",
    );
    expect(result.markdown).toContain("初始可见回复");
    expect(result.markdown).toContain("滚动后新增回复");
    expect(result.markdown).toContain("### 同作者续帖 1");
  });

  it("rejects shell pages that do not contain the requested status", () => {
    const { document } = parseHTML(`<main>${tweet("@other", "999", "推荐内容")}</main>`);
    expect(() => extractXThread(document as unknown as Document, "https://x.com/alice/status/100")).toThrow("登录壳");
    expect(isXStatusUrl("https://example.com/alice/status/100")).toBe(false);
    expect(isXStatusUrl("https://x.com/alice/status/100evil")).toBe(false);
  });
});
