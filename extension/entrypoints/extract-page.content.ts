import Defuddle from "defuddle";
import { MAX_HANDOFF_CONTENT_BYTES } from "../lib/handoff-transfer";
import { extractArxivHtml, isArxivHtmlUrl } from "../lib/arxiv-html-extractor";
import { extractXThread, extractXThreadSnapshots, isXStatusUrl } from "../lib/x-thread-extractor";

export type CapturedPage = {
  url: string;
  title: string;
  author: string | null;
  published: string | null;
  site: string | null;
  extractor: "defuddle" | "x-thread" | "arxiv-html" | "arxiv-pdf";
  markdown: string;
  word_count: number;
};

export type CapturedPageResult = CapturedPage | { error: string };

const MAX_SCROLL_ROUNDS = 6;
const MAX_SCROLL_HEIGHT = 100_000;
const LOAD_WAIT_MS = 250;
const MAX_X_EXPANSIONS = 12;
const MAX_X_POSTS = 80;

async function settleDynamicPage(maxRounds = MAX_SCROLL_ROUNDS): Promise<void> {
  let stableRounds = 0;
  let previousHeight = document.documentElement.scrollHeight;
  for (let round = 0; round < maxRounds && stableRounds < 2; round += 1) {
    const height = Math.min(document.documentElement.scrollHeight, MAX_SCROLL_HEIGHT);
    window.scrollTo({ top: height, behavior: "instant" });
    await new Promise((resolve) => window.setTimeout(resolve, LOAD_WAIT_MS));
    if (document.documentElement.scrollHeight > MAX_SCROLL_HEIGHT) throw new Error("页面长度超过当前提取边界，请缩小范围后重试");
    const currentHeight = document.documentElement.scrollHeight;
    stableRounds = Math.abs(currentHeight - previousHeight) < 8 ? stableRounds + 1 : 0;
    previousHeight = currentHeight;
  }
  if (stableRounds === 0 || document.documentElement.scrollHeight > MAX_SCROLL_HEIGHT) {
    throw new Error("页面仍在持续加载或长度超过当前提取边界，请稍后重试");
  }
}

function visibleXPostSignature(): string {
  return Array.from(document.querySelectorAll('article[data-testid="tweet"]'))
    .map((article) => article.querySelector("time")?.closest("a[href]")?.getAttribute("href") ?? article.textContent?.slice(0, 80) ?? "")
    .join("|");
}

function isXExpansionButton(button: HTMLElement): boolean {
  const label = (button.innerText || button.getAttribute("aria-label") || "").trim();
  return /^(show|view|显示|查看更多).*(repl|回复|thread|帖子)/i.test(label);
}

async function settleXThread(onSnapshot: () => void): Promise<void> {
  let expansions = 0;
  let stableRounds = 0;
  let previousSignature = visibleXPostSignature();
  const clickedButtons = new WeakSet<HTMLElement>();
  for (let round = 0; round < MAX_SCROLL_ROUNDS; round += 1) {
    let expandedThisRound = 0;
    const buttons = Array.from(document.querySelectorAll<HTMLElement>('button, [role="button"]'));
    for (const button of buttons) {
      if (expansions >= MAX_X_EXPANSIONS) break;
      if (clickedButtons.has(button) || button.getClientRects().length === 0 || button.getAttribute("aria-disabled") === "true") continue;
      if (!isXExpansionButton(button)) continue;
      clickedButtons.add(button);
      button.click();
      expansions += 1;
      expandedThisRound += 1;
    }
    const height = Math.min(document.documentElement.scrollHeight, MAX_SCROLL_HEIGHT);
    window.scrollTo({ top: height, behavior: "instant" });
    await new Promise((resolve) => window.setTimeout(resolve, LOAD_WAIT_MS));
    if (document.documentElement.scrollHeight > MAX_SCROLL_HEIGHT) throw new Error("X 线程长度超过当前提取边界，请缩小线程范围后重试");
    onSnapshot();
    const postCount = document.querySelectorAll('article[data-testid="tweet"]').length;
    if (postCount > MAX_X_POSTS) throw new Error(`可见帖子超过 ${MAX_X_POSTS} 条提取上限，请缩小线程范围后重试`);
    const hasUnexpandedButton = Array.from(document.querySelectorAll<HTMLElement>('button, [role="button"]'))
      .some((button) => !clickedButtons.has(button) && button.getClientRects().length > 0 && button.getAttribute("aria-disabled") !== "true" && isXExpansionButton(button));
    if (expansions >= MAX_X_EXPANSIONS && hasUnexpandedButton) {
      throw new Error(`X 线程达到 ${MAX_X_EXPANSIONS} 次展开上限，后续回复可能未包含`);
    }
    const signature = visibleXPostSignature();
    stableRounds = signature === previousSignature && expandedThisRound === 0 ? stableRounds + 1 : 0;
    previousSignature = signature;
    if (stableRounds >= 2) return;
  }
  throw new Error("X 线程在限定滚动次数内仍在加载，为避免交付不完整内容，请稍后重试");
}

export default defineContentScript({
  registration: "runtime",
  matches: ["http://*/*", "https://*/*"],
  async main(): Promise<CapturedPageResult> {
    const initialX = window.scrollX;
    const initialY = window.scrollY;
    try {
      if (["x.com", "www.x.com", "twitter.com", "www.twitter.com"].includes(location.hostname.toLowerCase()) && !isXStatusUrl(location.href)) {
        throw new Error("当前 X 页面不是单条帖子或线程，请先打开具体帖子后重试");
      }
      if (["arxiv.org", "www.arxiv.org"].includes(location.hostname.toLowerCase()) && location.pathname.startsWith("/html/") && !isArxivHtmlUrl(location.href)) {
        throw new Error("当前 arXiv HTML URL 不包含有效的论文标识");
      }
      if (isArxivHtmlUrl(location.href)) {
        const captured = extractArxivHtml(document.cloneNode(true) as Document, location.href);
        const byteLength = new TextEncoder().encode(captured.markdown).byteLength;
        if (byteLength > MAX_HANDOFF_CONTENT_BYTES) throw new Error("正文超过当前版本的 8 MiB 传输上限");
        return captured;
      }
      if (isXStatusUrl(location.href)) {
        // X 使用虚拟列表，滚动加载新回复时会卸载先前 DOM；按 permalink 合并每轮快照，
        // 避免“加载更多”反而丢掉已经可见的上文或回复。
        const snapshots = [document.cloneNode(true) as Document];
        let initialCapture: CapturedPage | null = null;
        try {
          initialCapture = extractXThread(document.cloneNode(true) as Document, location.href, MAX_X_POSTS);
        } catch {
          // 初始视口可能尚未包含当前 status，继续进行有界滚动后再判断。
        }
        let completenessWarning = "";
        try {
          await settleXThread(() => snapshots.push(document.cloneNode(true) as Document));
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          if (!initialCapture || message.includes("超过当前提取边界") || message.includes("超过 80 条")) throw error;
          completenessWarning = `> 完整性提示：${message}\n\n`;
        }
        let captured: CapturedPage | null = null;
        try {
          captured = extractXThreadSnapshots(snapshots, location.href, MAX_X_POSTS);
        } catch (error) {
          if (error instanceof Error && error.message.includes("超过")) throw error;
          // 初始快照仍是可信下限；只有跨快照合并失败时才回退。
          captured = initialCapture;
        }
        if (!captured) throw new Error("未找到当前帖子正文，页面可能只加载了推荐内容或登录壳");
        if (completenessWarning) {
          captured.markdown = completenessWarning + captured.markdown;
          captured.word_count += 1;
        }
        const byteLength = new TextEncoder().encode(captured.markdown).byteLength;
        if (byteLength > MAX_HANDOFF_CONTENT_BYTES) throw new Error("正文超过当前版本的 8 MiB 传输上限");
        return captured;
      }
      await settleDynamicPage();

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
    } catch (error) {
      return { error: error instanceof Error ? error.message : String(error) };
    } finally {
      // 用户阅读位置属于浏览器状态，即使提取失败也必须恢复。
      window.scrollTo({ left: initialX, top: initialY, behavior: "instant" });
    }
  },
});
