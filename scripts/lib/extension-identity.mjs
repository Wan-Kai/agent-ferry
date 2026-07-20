import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const EXTENSION_ID_PATTERN = /^[a-p]{32}$/;
const BASE64_PATTERN = /^[A-Za-z0-9+/]+={0,2}$/;

export function deriveExtensionId(manifestKey) {
  if (typeof manifestKey !== "string" || !BASE64_PATTERN.test(manifestKey)) {
    throw new Error("manifest_key 必须是无换行的 DER 公钥 Base64");
  }
  const publicKey = Buffer.from(manifestKey, "base64");
  if (publicKey.length === 0 || publicKey.toString("base64") !== manifestKey) {
    throw new Error("manifest_key 不是规范的 Base64");
  }

  // Chrome 用 SHA256 前 128 bit 的十六进制半字节映射 a-p 生成扩展 ID。
  // 在发行前自行推导并比对，避免手填 ID 与 Dashboard 公钥不属于同一个 Item。
  const prefix = createHash("sha256").update(publicKey).digest("hex").slice(0, 32);
  return [...prefix]
    .map((digit) => String.fromCharCode("a".charCodeAt(0) + Number.parseInt(digit, 16)))
    .join("");
}

export function parseExtensionIdentity(raw, source = "扩展身份文件") {
  let value;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    throw new Error(`${source} 不是有效 JSON: ${error.message}`);
  }

  if (value?.schema_version !== 1) {
    throw new Error(`${source} 的 schema_version 必须为 1`);
  }
  if (typeof value.extension_id !== "string" || !EXTENSION_ID_PATTERN.test(value.extension_id)) {
    throw new Error(`${source} 的 extension_id 必须是 32 位 a-p 字符`);
  }
  const derivedId = deriveExtensionId(value.manifest_key);
  if (derivedId !== value.extension_id) {
    throw new Error(
      `${source} 的 extension_id 与 manifest_key 不匹配：公钥推导结果为 ${derivedId}`,
    );
  }

  return Object.freeze({
    schema_version: 1,
    extension_id: value.extension_id,
    manifest_key: value.manifest_key,
  });
}

export function readExtensionIdentity(path) {
  return parseExtensionIdentity(readFileSync(path, "utf8"), path);
}
