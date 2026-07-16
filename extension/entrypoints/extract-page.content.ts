import Defuddle from "defuddle";
import { MAX_HANDOFF_CONTENT_BYTES } from "../lib/handoff-transfer";

export type CapturedPage = {
  url: string;
  title: string;
  author: string | null;
  published: string | null;
  site: string | null;
  extractor: "defuddle";
  markdown: string;
  word_count: number;
};

const MAX_SCROLL_ROUNDS = 6;
const MAX_SCROLL_HEIGHT = 100_000;
const LOAD_WAIT_MS = 250;

export default defineContentScript({
  registration: "runtime",
  matches: ["http://*/*", "https://*/*"],
  async main(): Promise<CapturedPage> {
    const initialX = window.scrollX;
    const initialY = window.scrollY;
    let stableRounds = 0;
    let previousHeight = document.documentElement.scrollHeight;
    try {
      for (let round = 0; round < MAX_SCROLL_ROUNDS && stableRounds < 2; round += 1) {
        const height = Math.min(document.documentElement.scrollHeight, MAX_SCROLL_HEIGHT);
        window.scrollTo({ top: height, behavior: "instant" });
        await new Promise((resolve) => window.setTimeout(resolve, LOAD_WAIT_MS));
        const currentHeight = document.documentElement.scrollHeight;
        stableRounds = Math.abs(currentHeight - previousHeight) < 8 ? stableRounds + 1 : 0;
        previousHeight = currentHeight;
      }
      if (stableRounds === 0 || document.documentElement.scrollHeight > MAX_SCROLL_HEIGHT) {
        throw new Error("页面仍在持续加载或长度超过当前提取边界，请稍后重试");
      }

      // Defuddle 会清理传入 DOM；使用快照避免提取过程改变用户正在阅读的页面。
      const snapshot = document.cloneNode(true) as Document;
      const parsed = new Defuddle(snapshot, {
        markdown: true,
        useAsync: false,
        url: location.href,
      }).parse();
      const markdown = (parsed.contentMarkdown ?? parsed.content ?? "").trim();
      const byteLength = new TextEncoder().encode(markdown).byteLength;
      if (markdown.length < 200 || parsed.wordCount < 40) {
        throw new Error("提取到的正文过短，可能仍未加载完整，请稍后重试");
      }
      if (byteLength > MAX_HANDOFF_CONTENT_BYTES) {
        throw new Error("正文超过当前版本的 8 MiB 传输上限");
      }
      return {
        url: location.href,
        title: parsed.title || document.title || location.hostname,
        author: parsed.author || null,
        published: parsed.published || null,
        site: parsed.site || parsed.domain || location.hostname,
        extractor: "defuddle",
        markdown,
        word_count: parsed.wordCount,
      };
    } finally {
      // 用户阅读位置属于浏览器状态，即使提取失败也必须恢复。
      window.scrollTo({ left: initialX, top: initialY, behavior: "instant" });
    }
  },
});
