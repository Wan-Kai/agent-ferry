import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const args = process.argv.slice(2);
const outputIndex = args.indexOf("--output");
if (outputIndex < 0 || !args[outputIndex + 1]) {
  throw new Error("证据写入缺少 --output <path>");
}

const outputPath = path.resolve(repositoryRoot, args[outputIndex + 1]);
const artifactPaths = [];
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "--artifact" && args[index + 1]) {
    artifactPaths.push(args[index + 1]);
    index += 1;
  }
}

const evidence = {
  schema_version: 1,
  verified_at: new Date().toISOString(),
  command: "./scripts/verify",
  result: "passed",
  git_commit: run("git", ["rev-parse", "HEAD"]),
  worktree_clean: run("git", ["status", "--porcelain"]) === "",
  toolchain: {
    rustc: run("rustc", ["--version"]),
    cargo: run("cargo", ["--version"]),
    node: process.version,
    npm: run("npm", ["--version"]),
  },
  artifacts: artifactPaths.map((artifact) => describeArtifact(artifact)),
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
console.log(`验证证据已写入 ${path.relative(repositoryRoot, outputPath)}`);

function describeArtifact(relativeArtifact) {
  const absoluteArtifact = path.resolve(repositoryRoot, relativeArtifact);
  const relative = path.relative(repositoryRoot, absoluteArtifact);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`产物必须位于仓库内：${relativeArtifact}`);
  }
  if (!fs.statSync(absoluteArtifact).isFile()) {
    throw new Error(`产物不是普通文件：${relativeArtifact}`);
  }
  const bytes = fs.readFileSync(absoluteArtifact);
  return {
    path: relative.split(path.sep).join("/"),
    bytes: bytes.length,
    sha256: crypto.createHash("sha256").update(bytes).digest("hex"),
  };
}

function run(command, commandArgs) {
  const result = spawnSync(command, commandArgs, { cwd: repositoryRoot, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`无法收集证据：${command} ${commandArgs.join(" ")}\n${result.stderr.trim()}`);
  }
  return result.stdout.trim();
}
