import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");

const result = spawnSync(
  "cargo",
  ["metadata", "--no-deps", "--format-version", "1"],
  { cwd: repositoryRoot, encoding: "utf8" },
);
if (result.status !== 0) {
  throw new Error(`无法读取 Cargo metadata：\n${result.stderr.trim()}`);
}

const metadata = JSON.parse(result.stdout);
const groups = new Map([
  ["agent-ferry-protocol", "foundation"],
  ["agent-ferry-transport", "foundation"],
  ["agent-ferry-core", "domain"],
  ["agent-ferry-claude", "adapter"],
  ["agent-ferry-codex", "adapter"],
  ["agent-ferry-hermes", "adapter"],
  ["agent-ferry-opencode", "adapter"],
  ["agent-ferry-daemon", "root"],
  ["agent-ferry-cli", "root"],
  ["agent-ferry-host", "root"],
]);

const workspacePackages = new Set(metadata.workspace_members);
const packagesById = new Map(metadata.packages.map((item) => [item.id, item]));
const workspaceNames = new Set(
  [...workspacePackages].map((id) => packagesById.get(id)?.name).filter(Boolean),
);

for (const name of workspaceNames) {
  if (!groups.has(name)) {
    fail(`未给 workspace crate ${name} 声明架构分层`);
  }
}
for (const name of groups.keys()) {
  if (!workspaceNames.has(name)) {
    fail(`架构规则包含不存在的 crate ${name}`);
  }
}

let checkedDependencies = 0;
for (const packageId of workspacePackages) {
  const sourcePackage = packagesById.get(packageId);
  const sourceGroup = groups.get(sourcePackage.name);
  for (const dependency of sourcePackage.dependencies) {
    if (!workspaceNames.has(dependency.name)) {
      continue;
    }
    checkedDependencies += 1;
    const targetGroup = groups.get(dependency.name);
    const dependencyKind = dependency.kind ?? "normal";

    if (sourceGroup === "foundation") {
      fail(`${sourcePackage.name} 作为 Foundation 不得依赖 ${dependency.name}`);
    }
    if (sourceGroup === "domain" && targetGroup !== "foundation") {
      fail(`${sourcePackage.name} 作为 Domain 只能依赖 Foundation，当前依赖 ${dependency.name}`);
    }
    if (sourceGroup === "adapter" && !["foundation", "domain"].includes(targetGroup)) {
      fail(`${sourcePackage.name} 作为 Adapter 不得依赖 ${dependency.name}`);
    }
    if (sourceGroup === "root" && targetGroup === "root" && dependencyKind !== "dev") {
      fail(`${sourcePackage.name} 不得生产依赖另一个组合根 ${dependency.name}`);
    }
  }
}

console.log(`架构依赖检查通过：${workspaceNames.size} 个 crate，${checkedDependencies} 条内部直接依赖`);

function fail(message) {
  throw new Error(`架构依赖检查失败：${message}`);
}
