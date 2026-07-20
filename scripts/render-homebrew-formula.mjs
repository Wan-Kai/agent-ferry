#!/usr/bin/env node

import fs from "node:fs";

const [templatePath, outputPath, version, arm64Url, arm64Sha, x86Url, x86Sha, extensionId] =
  process.argv.slice(2);

if (
  !templatePath ||
  !outputPath ||
  !version ||
  !arm64Url ||
  !arm64Sha ||
  !x86Url ||
  !x86Sha ||
  !extensionId
) {
  console.error(
    "用法: render-homebrew-formula.mjs <template> <output> <version> <arm64-url> <arm64-sha> <x86-url> <x86-sha> <extension-id>",
  );
  process.exit(2);
}

if (!/^\d+\.\d+\.\d+(?:[.-][A-Za-z0-9.-]+)?$/.test(version)) {
  throw new Error(`无效版本号：${version}`);
}
for (const [name, value] of [
  ["arm64 SHA256", arm64Sha],
  ["x86_64 SHA256", x86Sha],
]) {
  if (!/^[a-f0-9]{64}$/.test(value)) {
    throw new Error(`${name} 无效`);
  }
}
for (const [name, value] of [
  ["arm64 URL", arm64Url],
  ["x86_64 URL", x86Url],
]) {
  const url = new URL(value);
  if (!(["https:", "file:"].includes(url.protocol))) {
    throw new Error(`${name} 必须使用 https:// 或测试专用 file://`);
  }
}
if (!/^[a-p]{32}$/.test(extensionId)) {
  throw new Error("Chrome extension id 必须是 32 个 a-p 小写字符");
}

const replacements = new Map([
  ["__AGENT_FERRY_VERSION__", version],
  ["__AGENT_FERRY_ARM64_URL__", arm64Url],
  ["__AGENT_FERRY_ARM64_SHA256__", arm64Sha],
  ["__AGENT_FERRY_X86_64_URL__", x86Url],
  ["__AGENT_FERRY_X86_64_SHA256__", x86Sha],
  ["__AGENT_FERRY_EXTENSION_ID__", extensionId],
]);

let formula = fs.readFileSync(templatePath, "utf8");
for (const [placeholder, value] of replacements) {
  if (!formula.includes(placeholder)) {
    throw new Error(`Formula 模板缺少占位符：${placeholder}`);
  }
  formula = formula.replaceAll(placeholder, value);
}
if (/__AGENT_FERRY_[A-Z0-9_]+__/.test(formula)) {
  throw new Error("Formula 模板仍包含未渲染占位符");
}
fs.writeFileSync(outputPath, formula, { mode: 0o644 });
