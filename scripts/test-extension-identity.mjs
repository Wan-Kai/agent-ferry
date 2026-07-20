import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { deriveExtensionId, parseExtensionIdentity } from "./lib/extension-identity.mjs";

const manifestKey =
  "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA3CvNbwguuAemS1YdwNYfEzexT24vy2+FSgclJUQc1vsHkIpy3xBuWNJ6b2T3fKXfcyyWapQ/Q6pTf9VzSiCa98WSU0aPw1/xZWMRJkkXYaUnWno2fc8B9cD3KZjhs06SiYLozsWQIubPsEq6DMagu3qSiWmA9HPZz0bmtkiTYUrYJrfynoJMBrnoDMTqhzloeaI6GXqKKqYl3zFn3M7lZTXA5sifzAXKQvrIm631Onc3xEdzELCz4sgSpdaxSngly/Wzzw7v/Slo9avIww9QE168A7GelO6Xx1TmpJf+oOk6qhN5kbiKODuB6k1rG5RQHYJL+aoL7MA053VuEblcJQIDAQAB";
const extensionId = "bmbgkcbcohmlbiaigfoilnaobdoonkme";

assert.equal(deriveExtensionId(manifestKey), extensionId);
assert.deepEqual(
  parseExtensionIdentity(
    JSON.stringify({ schema_version: 1, extension_id: extensionId, manifest_key: manifestKey }),
  ),
  { schema_version: 1, extension_id: extensionId, manifest_key: manifestKey },
);
assert.throws(
  () =>
    parseExtensionIdentity(
      JSON.stringify({
        schema_version: 1,
        extension_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        manifest_key: manifestKey,
      }),
    ),
  /不匹配/,
);

const temporaryRoot = mkdtempSync(join(tmpdir(), "agent-ferry-extension-identity."));
try {
  const identityPath = join(temporaryRoot, "identity.json");
  writeFileSync(
    identityPath,
    `${JSON.stringify({ schema_version: 1, extension_id: extensionId, manifest_key: manifestKey })}\n`,
    { mode: 0o600 },
  );
  const result = spawnSync(
    process.execPath,
    ["scripts/extension-identity.mjs", identityPath, "extension_id"],
    { cwd: new URL("..", import.meta.url), encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), extensionId);
  assert.equal(readFileSync(identityPath, "utf8").includes(manifestKey), true);
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

console.log("Chrome 扩展身份校验测试通过");
