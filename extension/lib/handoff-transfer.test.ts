import { describe, expect, it } from "vitest";
import {
  MAX_HANDOFF_CHUNK_BYTES,
  MAX_HANDOFF_CONTENT_BYTES,
  prepareHandoffTransfer,
} from "./handoff-transfer";

describe("Handoff 分块", () => {
  it("跨 UTF-8 边界分块后可以逐字恢复并保持固定摘要", async () => {
    const markdown = `# 长文\n\n${"中文🙂与引号\"。".repeat(30_000)}`;
    const transfer = await prepareHandoffTransfer(markdown);
    expect(transfer.chunks.length).toBeGreaterThan(1);
    expect(transfer.chunks.join("")).toBe(markdown);
    expect(transfer.chunks.every((chunk) => new TextEncoder().encode(chunk).byteLength <= MAX_HANDOFF_CHUNK_BYTES)).toBe(true);
    expect(transfer.sha256).toMatch(/^[0-9a-f]{64}$/);
  });

  it("明确拒绝空正文和超过总量上限的正文", async () => {
    await expect(prepareHandoffTransfer("太短")).rejects.toThrow("过短");
    await expect(prepareHandoffTransfer("a".repeat(MAX_HANDOFF_CONTENT_BYTES + 1))).rejects.toThrow("8 MiB");
  });
});
