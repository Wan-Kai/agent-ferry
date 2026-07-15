import { defineConfig } from "wxt";

export default defineConfig({
  outDir: "dist",
  modules: ["@wxt-dev/module-react"],
  manifest: {
    name: "Agent Ferry",
    description: "Send web content to a local AI Agent workspace.",
    permissions: ["activeTab", "nativeMessaging"],
  },
});
