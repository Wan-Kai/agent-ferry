import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const expected = process.argv[2];
if (expected && !/^\d+\.\d+\.\d+(?:[-.][A-Za-z0-9.-]+)?$/.test(expected)) {
  throw new Error(`发行版本无效：${expected}`);
}

const metadataResult = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
  encoding: "utf8",
});
if (metadataResult.status !== 0) {
  throw new Error(`无法读取 Cargo metadata：${metadataResult.stderr.trim()}`);
}
const metadata = JSON.parse(metadataResult.stdout);
const versions = new Set(metadata.packages.map((item) => item.version));
const extensionVersion = JSON.parse(readFileSync("extension/package.json", "utf8")).version;
versions.add(extensionVersion);

if (versions.size !== 1) {
  throw new Error(`Rust workspace 与 Chrome 扩展版本不一致：${[...versions].join(", ")}`);
}
const actual = [...versions][0];
if (expected && actual !== expected) {
  throw new Error(`请求发行 ${expected}，但源码版本是 ${actual}`);
}
console.log(`发行版本一致：${actual}`);
