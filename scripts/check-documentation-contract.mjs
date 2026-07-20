import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");

const moduleMaps = ["", "crates", "crates/agent-ferry-daemon", "extension"];
for (const directory of moduleMaps) {
  const agentsPath = path.join(directory, "AGENTS.md");
  const claudePath = path.join(directory, "CLAUDE.md");
  const agents = read(agentsPath);
  assert(read(claudePath).trim() === "@AGENTS.md", `${claudePath} 只能包含 @AGENTS.md`);

  if (directory !== "") {
    for (const heading of [
      "## 职责",
      "## 关键成员",
      "## 依赖关系",
      "## 不变量",
      "## 变更影响",
      "## 验证",
      "## 关联文档",
    ]) {
      assert(agents.includes(heading), `${agentsPath} 缺少 ${heading}`);
    }
    assert(agents.includes("[PROTOCOL]"), `${agentsPath} 缺少文档回环标记 [PROTOCOL]`);
  }
}

const statusManaged = [
  "CONTEXT.md",
  "READE_DEV.md",
  "docs/README.md",
  "docs/documentation-lifecycle.md",
  "docs/glossary.md",
  ...listMarkdown("docs/architecture"),
  ...listMarkdown("docs/runbooks"),
  ...listMarkdown("docs/acceptance"),
  ...listMarkdown("docs/design"),
  ...listMarkdown("docs/prd"),
  ...listMarkdown("docs/research"),
];

for (const file of new Set(statusManaged)) {
  assert(
    /^> 状态：(Current|Draft|Historical|Superseded)/m.test(read(file)),
    `${file} 缺少受支持的文档状态`,
  );
}

for (const file of listMarkdown("docs/adr").filter((item) => item !== "docs/adr/README.md")) {
  const content = read(file);
  assert(content.includes("## 状态"), `${file} 缺少 ## 状态`);
  const statusSection = content.split("## 状态")[1]?.split("\n## ", 1)[0] ?? "";
  assert(
    /(Accepted|Superseded|Proposed|已接受|已被.+取代)/.test(statusSection),
    `${file} 的 ADR 状态无法识别`,
  );
}

const l3ContractFiles = [
  "crates/agent-ferry-protocol/src/lib.rs",
  "crates/agent-ferry-daemon/src/lib.rs",
  "crates/agent-ferry-daemon/src/history.rs",
  "crates/agent-ferry-hermes/src/lib.rs",
];
for (const file of l3ContractFiles) {
  const header = read(file).split("\n\n", 1)[0];
  assert(header.startsWith("//!"), `${file} 缺少文件头部 L3 契约`);
  for (const tag of ["[INPUT]", "[OUTPUT]", "[POS]", "[INVARIANTS]", "[PROTOCOL]"]) {
    assert(header.includes(tag), `${file} 的 L3 契约缺少 ${tag}`);
  }
}

console.log(`文档契约检查通过：${moduleMaps.length} 个上下文层级，${new Set(statusManaged).size} 份状态文档，${l3ContractFiles.length} 份 L3 契约`);

function listMarkdown(relativeDirectory) {
  const absoluteDirectory = path.join(repositoryRoot, relativeDirectory);
  if (!fs.existsSync(absoluteDirectory)) {
    return [];
  }
  return fs.readdirSync(absoluteDirectory, { withFileTypes: true }).flatMap((entry) => {
    const relativePath = path.join(relativeDirectory, entry.name);
    if (entry.isDirectory()) {
      return listMarkdown(relativePath);
    }
    return entry.isFile() && entry.name.endsWith(".md") ? [relativePath] : [];
  });
}

function read(relativePath) {
  const absolutePath = path.join(repositoryRoot, relativePath);
  assert(fs.existsSync(absolutePath), `缺少 ${relativePath}`);
  return fs.readFileSync(absolutePath, "utf8");
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`文档契约检查失败：${message}`);
  }
}
