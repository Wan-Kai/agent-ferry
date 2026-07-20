#!/usr/bin/env node

import { readExtensionIdentity } from "./lib/extension-identity.mjs";

const [path, field] = process.argv.slice(2);
if (!path || ![undefined, "extension_id", "manifest_key"].includes(field)) {
  console.error("用法：extension-identity.mjs <identity.json> [extension_id|manifest_key]");
  process.exit(2);
}

try {
  const identity = readExtensionIdentity(path);
  process.stdout.write(field ? `${identity[field]}\n` : `${JSON.stringify(identity)}\n`);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
