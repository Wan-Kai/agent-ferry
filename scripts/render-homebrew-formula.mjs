#!/usr/bin/env node

import fs from "node:fs";

const args = process.argv.slice(2);
const [templatePath, outputPath, version, arm64Url, arm64Sha, x86Url, x86Sha, extensionId] =
  args.slice(0, 8);
const optionalArgs = args.slice(8);

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
    "用法: render-homebrew-formula.mjs <template> <output> <version> <arm64-url> <arm64-sha> <x86-url> <x86-sha> <extension-id> [Bottle 参数]",
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

const bottleOptions = new Map();
for (let index = 0; index < optionalArgs.length; index += 2) {
  const name = optionalArgs[index];
  const value = optionalArgs[index + 1];
  if (!name?.startsWith("--") || value === undefined) {
    throw new Error(`无效 Bottle 参数：${name ?? "<缺失>"}`);
  }
  if (bottleOptions.has(name)) {
    throw new Error(`重复 Bottle 参数：${name}`);
  }
  bottleOptions.set(name, value);
}

const bottleNames = [
  "--bottle-root-url",
  "--arm64-bottle-tag",
  "--arm64-bottle-sha",
  "--x86-64-bottle-tag",
  "--x86-64-bottle-sha",
];
const unknownBottleNames = [...bottleOptions.keys()].filter((name) => !bottleNames.includes(name));
if (unknownBottleNames.length > 0) {
  throw new Error(`未知 Bottle 参数：${unknownBottleNames.join(", ")}`);
}
const hasBottle = bottleOptions.size > 0;
if (hasBottle && bottleNames.some((name) => !bottleOptions.has(name))) {
  throw new Error("Bottle 参数必须同时提供 root URL、双架构 tag 与 SHA256");
}

let bottleBlock = "";
if (hasBottle) {
  const rootUrl = bottleOptions.get("--bottle-root-url");
  const root = new URL(rootUrl);
  if (!(root.protocol === "https:" || root.protocol === "file:")) {
    throw new Error("Bottle root URL 必须使用 https:// 或测试专用 file://");
  }
  const arm64Tag = bottleOptions.get("--arm64-bottle-tag");
  const x86Tag = bottleOptions.get("--x86-64-bottle-tag");
  if (!/^arm64_[a-z][a-z0-9_]*$/.test(arm64Tag)) {
    throw new Error(`无效 arm64 Bottle tag：${arm64Tag}`);
  }
  if (!/^[a-z][a-z0-9_]*$/.test(x86Tag) || x86Tag.startsWith("arm64_")) {
    throw new Error(`无效 x86_64 Bottle tag：${x86Tag}`);
  }
  for (const [name, value] of [
    ["arm64 Bottle SHA256", bottleOptions.get("--arm64-bottle-sha")],
    ["x86_64 Bottle SHA256", bottleOptions.get("--x86-64-bottle-sha")],
  ]) {
    if (!/^[a-f0-9]{64}$/.test(value)) {
      throw new Error(`${name} 无效`);
    }
  }

  bottleBlock = [
    "  bottle do",
    `    root_url ${JSON.stringify(rootUrl)}`,
    `    sha256 cellar: :any_skip_relocation, ${arm64Tag}: ${JSON.stringify(bottleOptions.get("--arm64-bottle-sha"))}`,
    `    sha256 cellar: :any_skip_relocation, ${x86Tag}: ${JSON.stringify(bottleOptions.get("--x86-64-bottle-sha"))}`,
    "  end",
  ].join("\n");
}

const replacements = new Map([
  ["__AGENT_FERRY_VERSION__", version],
  ["__AGENT_FERRY_ARM64_URL__", arm64Url],
  ["__AGENT_FERRY_ARM64_SHA256__", arm64Sha],
  ["__AGENT_FERRY_X86_64_URL__", x86Url],
  ["__AGENT_FERRY_X86_64_SHA256__", x86Sha],
  ["__AGENT_FERRY_EXTENSION_ID__", extensionId],
  ["__AGENT_FERRY_BOTTLE_BLOCK__", bottleBlock],
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
