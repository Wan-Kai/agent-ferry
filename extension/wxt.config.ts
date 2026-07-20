import { defineConfig } from "wxt";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

const identityPath = process.env.AGENT_FERRY_EXTENSION_IDENTITY;
const identityRequired = process.env.AGENT_FERRY_REQUIRE_EXTENSION_IDENTITY === "1";
let manifestKey: string | undefined;

if (identityRequired && !identityPath) {
  throw new Error("发行构建必须设置 AGENT_FERRY_EXTENSION_IDENTITY");
}
if (identityPath) {
  const validator = resolve(import.meta.dirname, "../scripts/extension-identity.mjs");
  manifestKey = execFileSync(process.execPath, [validator, resolve(identityPath), "manifest_key"], {
    encoding: "utf8",
  }).trim();
}

export default defineConfig({
  outDir: "dist",
  modules: ["@wxt-dev/module-react"],
  manifest: {
    name: "Agent Ferry",
    description: "Send the current web page and your visible prompt to a local AI Agent or your own remote Hermes.",
    icons: {
      16: "icons/icon-16.png",
      32: "icons/icon-32.png",
      48: "icons/icon-48.png",
      128: "icons/icon-128.png",
    },
    action: {
      default_icon: {
        16: "icons/icon-16.png",
        32: "icons/icon-32.png",
        48: "icons/icon-48.png",
        128: "icons/icon-128.png",
      },
    },
    permissions: ["activeTab", "nativeMessaging", "scripting", "storage"],
    ...(manifestKey ? { key: manifestKey } : {}),
  },
});
