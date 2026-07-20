#!/usr/bin/env node
import fs from "node:fs";

const [templatePath, outputPath, teamId] = process.argv.slice(2);
if (!templatePath || !outputPath || !/^[A-Z0-9]{10}$/.test(teamId ?? "")) {
  console.error("用法：render-installer.mjs <template> <output> <10位 Apple Team ID>");
  process.exit(2);
}

const placeholder = "__AGENT_FERRY_SIGNING_TEAM_ID__";
const template = fs.readFileSync(templatePath, "utf8");
const occurrences = template.split(placeholder).length - 1;
if (occurrences !== 1) {
  throw new Error(`安装器模板中的 Team ID 占位符数量应为 1，实际为 ${occurrences}`);
}
fs.writeFileSync(outputPath, template.replace(placeholder, teamId), { mode: 0o755 });
