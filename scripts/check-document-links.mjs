import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const markdownFiles = [
  "README.md",
  "READE_DEV.md",
  "AGENTS.md",
  "CONTEXT.md",
  ...listMarkdown("docs"),
  ...listNamedMarkdown("crates", new Set(["AGENTS.md", "CLAUDE.md"])),
  ...listNamedMarkdown("extension", new Set(["AGENTS.md", "CLAUDE.md"]), new Set(["node_modules", ".wxt", "dist"])),
];

const failures = [];
let checkedLinks = 0;
for (const source of new Set(markdownFiles)) {
  const content = fs.readFileSync(path.join(repositoryRoot, source), "utf8");
  const pattern = /!?\[[^\]]*\]\(([^)]+)\)/g;
  for (const match of content.matchAll(pattern)) {
    const rawTarget = match[1].trim().replace(/^<|>$/g, "");
    const target = rawTarget.split(/\s+["']/)[0];
    if (shouldIgnore(target)) {
      continue;
    }
    checkedLinks += 1;
    const pathPart = target.split("#")[0].split("?")[0];
    if (!pathPart) {
      continue;
    }
    let decoded;
    try {
      decoded = decodeURIComponent(pathPart);
    } catch {
      failures.push(`${source} 包含无法解码的链接：${target}`);
      continue;
    }
    const absoluteTarget = path.resolve(path.dirname(path.join(repositoryRoot, source)), decoded);
    const relativeTarget = path.relative(repositoryRoot, absoluteTarget);
    if (relativeTarget.startsWith("..") || path.isAbsolute(relativeTarget)) {
      failures.push(`${source} 链接越出仓库：${target}`);
      continue;
    }
    if (!fs.existsSync(absoluteTarget)) {
      failures.push(`${source} 链接不存在：${target}`);
    }
  }
}

if (failures.length > 0) {
  throw new Error(`文档链接检查失败：\n${failures.map((item) => `- ${item}`).join("\n")}`);
}
console.log(`文档链接检查通过：${new Set(markdownFiles).size} 个文件，${checkedLinks} 个本地链接`);

function listMarkdown(relativeDirectory) {
  return listNamedMarkdown(relativeDirectory, undefined, new Set(["node_modules", ".wxt", "dist", "target"]));
}

function listNamedMarkdown(relativeDirectory, names, ignoredDirectories = new Set()) {
  const absoluteDirectory = path.join(repositoryRoot, relativeDirectory);
  if (!fs.existsSync(absoluteDirectory)) {
    return [];
  }
  return fs.readdirSync(absoluteDirectory, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isDirectory()) {
      return ignoredDirectories.has(entry.name)
        ? []
        : listNamedMarkdown(path.join(relativeDirectory, entry.name), names, ignoredDirectories);
    }
    if (!entry.isFile() || !entry.name.endsWith(".md") || (names && !names.has(entry.name))) {
      return [];
    }
    return [path.join(relativeDirectory, entry.name)];
  });
}

function shouldIgnore(target) {
  return target.startsWith("#")
    || target.startsWith("http://")
    || target.startsWith("https://")
    || target.startsWith("mailto:")
    || target.startsWith("app://");
}
