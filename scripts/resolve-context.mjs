import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const targets = process.argv.slice(2);

if (targets.length === 0) {
  console.error("用法：./scripts/context <目标文件或目录...>");
  process.exit(1);
}

const contextFiles = new Set([
  path.join(repositoryRoot, "AGENTS.md"),
  path.join(repositoryRoot, "CONTEXT.md"),
  path.join(repositoryRoot, "docs/glossary.md"),
  path.join(repositoryRoot, "docs/architecture/overview.md"),
  path.join(repositoryRoot, "docs/architecture/dependency-rules.md"),
]);

for (const target of targets) {
  const targetDirectory = resolveTargetDirectory(target);
  for (const directory of directoriesFromRoot(targetDirectory)) {
    const agentsFile = path.join(directory, "AGENTS.md");
    if (fs.existsSync(agentsFile)) {
      contextFiles.add(agentsFile);
    }
  }
}

for (const file of contextFiles) {
  if (!fs.existsSync(file)) {
    throw new Error(`上下文文件不存在：${toRelative(file)}`);
  }
  process.stdout.write(`\n===== CONTEXT: ${toRelative(file)} =====\n\n`);
  process.stdout.write(fs.readFileSync(file, "utf8").trimEnd());
  process.stdout.write("\n");
}

function resolveTargetDirectory(target) {
  const absoluteTarget = path.resolve(repositoryRoot, target);
  assertInsideRepository(absoluteTarget, target);

  if (fs.existsSync(absoluteTarget)) {
    return fs.statSync(absoluteTarget).isDirectory()
      ? absoluteTarget
      : path.dirname(absoluteTarget);
  }

  let cursor = path.dirname(absoluteTarget);
  while (!fs.existsSync(cursor)) {
    const parent = path.dirname(cursor);
    if (parent === cursor) {
      throw new Error(`无法解析目标路径：${target}`);
    }
    cursor = parent;
  }
  assertInsideRepository(cursor, target);
  return fs.statSync(cursor).isDirectory() ? cursor : path.dirname(cursor);
}

function assertInsideRepository(absolutePath, originalTarget) {
  const relativePath = path.relative(repositoryRoot, absolutePath);
  if (relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
    throw new Error(`目标路径必须位于仓库内：${originalTarget}`);
  }
}

function directoriesFromRoot(targetDirectory) {
  const relativeDirectory = path.relative(repositoryRoot, targetDirectory);
  if (!relativeDirectory) {
    return [repositoryRoot];
  }

  const directories = [repositoryRoot];
  let cursor = repositoryRoot;
  for (const segment of relativeDirectory.split(path.sep)) {
    cursor = path.join(cursor, segment);
    directories.push(cursor);
  }
  return directories;
}

function toRelative(absolutePath) {
  return path.relative(repositoryRoot, absolutePath).split(path.sep).join("/") || ".";
}
