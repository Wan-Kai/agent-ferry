import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(scriptDirectory, "..");

assertPng("extension/public/icons/icon-128.png", 128, 128);
assertPng("extension/public/icons/icon-512.png", 512, 512);
assertPng("release/chrome-store/promo-small.png", 440, 280);

const privacy = read("PRIVACY.md");
for (const required of [
  "website content and browsing context",
  "user-generated task instructions",
  "does not operate a developer-owned service",
  "Chrome Web Store User Data Policy",
  "aferry uninstall --purge --yes",
]) {
  assert(privacy.includes(required), `隐私政策缺少披露：${required}`);
}

const submission = read("docs/runbooks/chrome-web-store.md");
for (const required of [
  "`activeTab`",
  "`scripting`",
  "`nativeMessaging`",
  "`storage`",
  "`Website content`",
  "`Web history`",
  "`User-generated content`",
  "https://github.com/Wan-Kai/agent-ferry/blob/main/PRIVACY.md",
]) {
  assert(submission.includes(required), `商店提交说明缺少：${required}`);
}

console.log("Chrome Web Store 隐私披露与静态图像契约检查通过");

function assertPng(relativePath, width, height) {
  const bytes = fs.readFileSync(path.join(root, relativePath));
  const signature = bytes.subarray(0, 8).toString("hex");
  assert(signature === "89504e470d0a1a0a", `${relativePath} 不是 PNG`);
  assert(bytes.length >= 24, `${relativePath} PNG 头不完整`);
  const actualWidth = bytes.readUInt32BE(16);
  const actualHeight = bytes.readUInt32BE(20);
  assert(actualWidth === width && actualHeight === height, `${relativePath} 尺寸应为 ${width}x${height}，实际为 ${actualWidth}x${actualHeight}`);
}

function read(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function assert(condition, message) {
  if (!condition) throw new Error(`商店就绪检查失败：${message}`);
}
