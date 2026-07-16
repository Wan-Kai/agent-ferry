export const MAX_HANDOFF_CHUNK_BYTES = 192 * 1024;
export const MAX_HANDOFF_CONTENT_BYTES = 8 * 1024 * 1024;
export const MIN_HANDOFF_CONTENT_BYTES = 200;
export const MIN_X_HANDOFF_CONTENT_BYTES = 40;
const HANDOFF_CHUNK_INPUT_BYTES = MAX_HANDOFF_CHUNK_BYTES - 4;

export type PreparedHandoffTransfer = {
  totalBytes: number;
  sha256: string;
  chunks: string[];
};

export async function prepareHandoffTransfer(markdown: string, minimumBytes = MIN_HANDOFF_CONTENT_BYTES): Promise<PreparedHandoffTransfer> {
  const bytes = new TextEncoder().encode(markdown);
  if (bytes.byteLength < minimumBytes) throw new Error("提取到的正文过短，不能创建 Handoff");
  if (bytes.byteLength > MAX_HANDOFF_CONTENT_BYTES) throw new Error("正文超过当前版本的 8 MiB 传输上限");

  const decoder = new TextDecoder("utf-8", { fatal: true });
  const chunks: string[] = [];
  for (let offset = 0; offset < bytes.byteLength; offset += HANDOFF_CHUNK_INPUT_BYTES) {
    const end = Math.min(offset + HANDOFF_CHUNK_INPUT_BYTES, bytes.byteLength);
    // streaming decoder 会把跨 chunk 的多字节字符留到下一帧，避免替换字符破坏 sha256。
    const chunk = decoder.decode(bytes.subarray(offset, end), { stream: end < bytes.byteLength });
    if (chunk) chunks.push(chunk);
  }
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const sha256 = Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return { totalBytes: bytes.byteLength, sha256, chunks };
}
