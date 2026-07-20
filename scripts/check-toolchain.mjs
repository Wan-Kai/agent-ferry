import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const cargoManifest = fs.readFileSync(path.join(repositoryRoot, "Cargo.toml"), "utf8");
const minimumRust = cargoManifest.match(/^rust-version\s*=\s*"([^"]+)"/m)?.[1];
if (!minimumRust) {
  throw new Error("工具链检查失败：Cargo.toml 缺少 workspace rust-version");
}

const rustVersion = commandVersion("rustc", ["--version"]).match(/rustc\s+(\d+\.\d+\.\d+)/)?.[1];
if (!rustVersion || compareVersions(rustVersion, minimumRust) < 0) {
  throw new Error(`工具链检查失败：需要 Rust >= ${minimumRust}，当前为 ${rustVersion ?? "unknown"}`);
}

const nodeMajor = Number(process.versions.node.split(".")[0]);
if (nodeMajor !== 22) {
  throw new Error(`工具链检查失败：需要 Node 22，当前为 ${process.versions.node}`);
}

const npmVersion = commandVersion("npm", ["--version"]);
console.log(`工具链检查通过：Rust ${rustVersion}（MSRV ${minimumRust}），Node ${process.versions.node}，npm ${npmVersion}`);

function commandVersion(command, args) {
  const result = spawnSync(command, args, { cwd: repositoryRoot, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`工具链检查失败：无法执行 ${command} ${args.join(" ")}\n${result.stderr.trim()}`);
  }
  return result.stdout.trim();
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < Math.max(leftParts.length, rightParts.length); index += 1) {
    const difference = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (difference !== 0) {
      return difference;
    }
  }
  return 0;
}
