export type XThreadCapture = {
  url: string;
  title: string;
  author: string | null;
  published: string | null;
  site: "X (Twitter)";
  extractor: "x-thread";
  markdown: string;
  word_count: number;
};

type XPost = {
  displayName: string;
  handle: string;
  published: string | null;
  permalink: string;
  text: string;
  media: Array<{ src: string; alt: string }>;
};

const X_HOSTS = new Set(["x.com", "www.x.com", "twitter.com", "www.twitter.com"]);

export function isXStatusUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return X_HOSTS.has(url.hostname.toLowerCase()) && /^\/[^/]+\/status\/\d+(?:\/|$)/.test(url.pathname);
  } catch {
    return false;
  }
}

function statusId(value: string): string | null {
  try {
    return new URL(value).pathname.match(/\/status\/(\d+)/)?.[1] ?? null;
  } catch {
    return null;
  }
}

function absoluteUrl(value: string | null, baseUrl: string): string {
  if (!value) return "";
  try {
    return new URL(value, baseUrl).href;
  } catch {
    return "";
  }
}

function normalizedText(value: string | null | undefined): string {
  return (value ?? "")
    .replace(/\u00a0/g, " ")
    .split(/\n+/)
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .join("\n");
}

function countWords(value: string): number {
  const cjk = value.match(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g)?.length ?? 0;
  const words = value.replace(/[\u3040-\u30ff\u3400-\u9fff\uf900-\ufaff\uac00-\ud7af]/g, " ").match(/[\p{L}\p{N}_]+/gu)?.length ?? 0;
  return cjk + words;
}

function parsePost(article: Element, baseUrl: string): XPost | null {
  const name = article.querySelector('[data-testid="User-Name"]');
  const links = Array.from(name?.querySelectorAll("a[href]") ?? []);
  const handleLink = links.find((link) => normalizedText(link.textContent).startsWith("@"));
  const handle = normalizedText(handleLink?.textContent) || (handleLink?.getAttribute("href")?.split("/").filter(Boolean)[0] ? `@${handleLink.getAttribute("href")?.split("/").filter(Boolean)[0]}` : "");
  const displayName = normalizedText(links.find((link) => link !== handleLink)?.textContent) || handle;
  const time = article.querySelector("time");
  const permalink = absoluteUrl(time?.closest("a[href]")?.getAttribute("href") ?? null, baseUrl);
  const text = normalizedText(article.querySelector('[data-testid="tweetText"]')?.textContent);
  if (!text && !article.querySelector('[data-testid="tweetPhoto"], [data-testid="tweet-image"]')) return null;

  const seenMedia = new Set<string>();
  const media = Array.from(article.querySelectorAll('[data-testid="tweetPhoto"] img, [data-testid="tweet-image"] img, img[src*="media"]'))
    .map((image) => ({
      src: absoluteUrl(image.getAttribute("src"), baseUrl),
      alt: normalizedText(image.getAttribute("alt")) || "帖子图片",
    }))
    .filter((image) => image.src && !seenMedia.has(image.src) && seenMedia.add(image.src));
  return {
    displayName,
    handle,
    published: time?.getAttribute("datetime") ?? null,
    permalink,
    text,
    media,
  };
}

function formatPost(post: XPost, heading: string, depth = 0): string {
  const author = [post.displayName, post.handle].filter(Boolean).join(" ") || "未知作者";
  const lines = [
    `### ${heading}`,
    "",
    `- 作者：${author}`,
    `- 时间：${post.published ?? "未知"}`,
    `- 链接：${post.permalink || "未知"}`,
    "",
    post.text,
    ...post.media.map((media) => `![${media.alt}](${media.src})`),
  ];
  const prefix = "> ".repeat(depth);
  return depth === 0 ? lines.join("\n") : lines.map((line) => `${prefix}${line}`.trimEnd()).join("\n");
}

/**
 * 背景与目的：X 的页面是动态 timeline，通用正文算法容易只拿到登录壳或把推荐帖混入正文。
 * 这里仅消费页面中已经可见的 tweet article，并要求当前 status 的永久链接确实存在。
 *
 * 设计理由与约束：
 * - DOM 分类沿用 Defuddle Twitter extractor 的思路，但输出逐帖元数据，避免依赖其内部未公开模块。
 * - X 没有稳定公开的回复深度字段；连续 cell 只保留保守的可见层级，不能推断不可见关系。
 */
function collectPosts(document: Document, pageUrl: string): XPost[] {
  const timeline = document.querySelector('[aria-label="Timeline: Conversation"], [aria-label*="Conversation"]');
  const cells = timeline
    ? Array.from(timeline.querySelectorAll('[data-testid="cellInnerDiv"]'))
    : Array.from(document.querySelectorAll('article[data-testid="tweet"]')).map((article) => article.parentElement ?? article);
  return cells
    .map((cell) => cell.querySelector('article[data-testid="tweet"]'))
    .map((article) => article ? parsePost(article, pageUrl) : null)
    .filter((post): post is XPost => post !== null);
}

export function extractXThread(document: Document, pageUrl: string, maxPosts = 80): XThreadCapture {
  return extractXThreadSnapshots([document], pageUrl, maxPosts);
}

/** 合并虚拟列表在不同滚动位置暴露的帖子，permalink 是唯一稳定的跨快照标识。 */
export function extractXThreadSnapshots(documents: readonly Document[], pageUrl: string, maxPosts = 80): XThreadCapture {
  if (!isXStatusUrl(pageUrl)) throw new Error("当前页面不是可提取的 X/Twitter 帖子页面");
  const rootId = statusId(pageUrl);
  const posts: XPost[] = [];
  const seen = new Set<string>();
  const postKey = (post: XPost) => post.permalink || `${post.handle}\n${post.published ?? ""}\n${post.text}`;
  for (const document of documents) {
    const snapshot = collectPosts(document, pageUrl);
    for (let index = 0; index < snapshot.length;) {
      if (seen.has(postKey(snapshot[index]))) {
        index += 1;
        continue;
      }
      const start = index;
      while (index < snapshot.length && !seen.has(postKey(snapshot[index]))) index += 1;
      const run = snapshot.slice(start, index);
      const nextKnownKey = index < snapshot.length ? postKey(snapshot[index]) : null;
      const previousKnownKey = start > 0 ? postKey(snapshot[start - 1]) : null;
      const insertionIndex = nextKnownKey
        ? posts.findIndex((post) => postKey(post) === nextKnownKey)
        : previousKnownKey
          ? posts.findIndex((post) => postKey(post) === previousKnownKey) + 1
          : posts.length;
      posts.splice(Math.max(0, insertionIndex), 0, ...run);
      run.forEach((post) => seen.add(postKey(post)));
      if (posts.length > maxPosts) throw new Error(`可见帖子超过 ${maxPosts} 条提取上限，请缩小线程范围后重试`);
    }
  }
  if (posts.length === 0) throw new Error("X 页面尚未加载出帖子正文，请等待页面完成加载后重试");
  const targetIndex = posts.findIndex((post) => statusId(post.permalink) === rootId);
  if (targetIndex < 0) throw new Error("未找到当前帖子正文，页面可能只加载了推荐内容或登录壳");

  const target = posts[targetIndex];
  let threadStart = targetIndex;
  while (threadStart > 0 && posts[threadStart - 1].handle === target.handle) threadStart -= 1;
  let threadEnd = targetIndex + 1;
  while (threadEnd < posts.length && posts[threadEnd].handle === target.handle) threadEnd += 1;
  const context = posts.slice(0, threadStart);
  const thread = posts.slice(threadStart, threadEnd);
  const replies = posts.slice(threadEnd);

  const titleAuthor = target.handle || target.displayName || "未知作者";
  const sections = [`# X 对话：${titleAuthor}`];
  if (context.length > 0) {
    sections.push("", "## 可见上文");
    context.forEach((post, index) => sections.push("", formatPost(post, `上文 ${index + 1}`)));
  }
  sections.push("", "## 同作者线程");
  thread.forEach((post, index) => {
    const current = post.permalink === target.permalink ? "（当前页面）" : "";
    sections.push("", formatPost(post, `${index === 0 ? "主帖" : `同作者续帖 ${index}`}${current}`));
  });
  if (replies.length > 0) {
    sections.push("", "## 可见回复");
    // X 当前 DOM 没有稳定回复深度字段；统一保留为一层引用，避免把连续平级回复伪造成嵌套关系。
    replies.forEach((post, index) => sections.push("", formatPost(post, `回复 ${index + 1}`, 1)));
  }
  const markdown = sections.join("\n").trim();
  return {
    url: pageUrl,
    title: `X 对话：${titleAuthor}`,
    author: [target.displayName, target.handle].filter(Boolean).join(" ") || null,
    published: target.published,
    site: "X (Twitter)",
    extractor: "x-thread",
    markdown,
    word_count: countWords(markdown),
  };
}
