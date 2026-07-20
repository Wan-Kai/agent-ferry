#!/usr/bin/env bash
set -euo pipefail

manifest_url="https://github.com/Wan-Kai/agent-ferry/releases/latest/download/release-manifest.json"
extension_id=""
expected_team_id="__AGENT_FERRY_SIGNING_TEAM_ID__"
codesign_bin="/usr/bin/codesign"

if [[ "${AGENT_FERRY_INSTALLER_TEST_MODE:-}" == "1" ]]; then
  expected_team_id="${AGENT_FERRY_EXPECTED_TEAM_ID:?测试模式必须提供 AGENT_FERRY_EXPECTED_TEAM_ID}"
  codesign_bin="${AGENT_FERRY_CODESIGN_BIN:?测试模式必须提供 AGENT_FERRY_CODESIGN_BIN}"
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-url)
      manifest_url="${2:-}"
      shift 2
      ;;
    --extension-id)
      extension_id="${2:-}"
      shift 2
      ;;
    *)
      printf '未知参数：%s\n' "$1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf '%s\n' 'Agent Ferry 当前安装器只支持 macOS。' >&2
  exit 1
fi
if [[ ! "${expected_team_id}" =~ ^[A-Z0-9]{10}$ ]]; then
  printf '%s\n' '安装器尚未绑定正式 Apple Team ID，请使用 GitHub Release 中的 install.sh。' >&2
  exit 1
fi
architecture="$(uname -m)"
case "${architecture}" in
  arm64 | x86_64) ;;
  *)
    printf '不支持的 macOS 架构：%s\n' "${architecture}" >&2
    exit 1
    ;;
esac
if [[ -n "${extension_id}" && ! "${extension_id}" =~ ^[a-p]{32}$ ]]; then
  printf 'Chrome extension id 无效：%s\n' "${extension_id}" >&2
  exit 2
fi

install_root="${HOME}/.local/share/agent-ferry"
versions_root="${install_root}/versions"
command_root="${HOME}/.local/bin"
current_link="${install_root}/current"
lock_directory="${HOME}/.local/share/.agent-ferry.lock"
temporary_root=""
target_backup=""
previous_current=""
current_switched="false"
target_installed="false"
data_migrated="false"
native_manifest_touched="false"
committed="false"
old_plist_snapshot=""
old_manifest_snapshot=""

cleanup() {
  local exit_code=$?
  if [[ "${committed}" != "true" ]]; then
    set +e
    if [[ "${current_switched}" == "true" ]]; then
      "${install_root}/current/bin/aferry" service uninstall >/dev/null 2>&1
      rm -f "${current_link}"
      if [[ -n "${previous_current}" ]]; then
        ln -s "${previous_current}" "${current_link}"
      else
        for binary in aferry agentferryd agentferry-host; do
          command_path="${command_root}/${binary}"
          if [[ -L "${command_path}" && "$(readlink "${command_path}")" == "${install_root}/current/bin/${binary}" ]]; then
            rm -f "${command_path}"
          fi
        done
      fi
    fi
    if [[ -n "${target_backup}" && -e "${target_backup}" ]]; then
      rm -rf "${target_version:-}"
      mv "${target_backup}" "${target_version}"
    fi
    if [[ "${data_migrated}" == "true" && -d "${HOME}/.agent-ferry" ]]; then
      legacy_root="${HOME}/Library/Application Support/Agent Ferry"
      if [[ ! -e "${legacy_root}" ]]; then
        mkdir -p "$(dirname "${legacy_root}")"
        mv "${HOME}/.agent-ferry" "${legacy_root}"
      fi
    fi
    native_manifest="${HOME}/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.agentferry.host.json"
    if [[ -n "${old_manifest_snapshot}" && -f "${old_manifest_snapshot}" ]]; then
      cp "${old_manifest_snapshot}" "${native_manifest}"
      chmod 600 "${native_manifest}"
    elif [[ "${native_manifest_touched}" == "true" ]]; then
      rm -f "${native_manifest}"
    fi
    if [[ -n "${old_plist_snapshot}" && -f "${old_plist_snapshot}" ]]; then
      cp "${old_plist_snapshot}" "${HOME}/Library/LaunchAgents/com.agentferry.daemon.plist"
      chmod 600 "${HOME}/Library/LaunchAgents/com.agentferry.daemon.plist"
      if [[ -n "${previous_current}" && -x "${install_root}/${previous_current}/bin/aferry" ]]; then
        "${install_root}/${previous_current}/bin/aferry" service start >/dev/null 2>&1
      elif [[ -n "${new_aferry:-}" && -x "${new_aferry}" ]]; then
        "${new_aferry}" service start >/dev/null 2>&1
      fi
    fi
    if [[ "${target_installed}" == "true" && -z "${target_backup}" ]]; then
      rm -rf "${target_version:-}"
    fi
  fi
  [[ -z "${temporary_root}" ]] || rm -rf "${temporary_root}"
  rmdir "${lock_directory}" 2>/dev/null || true
  exit "${exit_code}"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ -L "${install_root}" ]]; then
  printf '安装根目录不能是符号链接：%s\n' "${install_root}" >&2
  exit 1
fi
mkdir -p "${HOME}/.local/share" "${command_root}"
if ! mkdir "${lock_directory}" 2>/dev/null; then
  printf '%s\n' '已有另一个 Agent Ferry 安装、更新或卸载正在执行。' >&2
  exit 1
fi
chmod 700 "${lock_directory}"
mkdir -p "${install_root}" "${versions_root}"
chmod 700 "${install_root}"
temporary_root="$(mktemp -d "${install_root}/.staging.XXXXXX")"

manifest="${temporary_root}/release-manifest.json"
/usr/bin/curl -fsSL --proto '=https,file' --tlsv1.2 "${manifest_url}" -o "${manifest}"
manifest_value() {
  /usr/bin/plutil -extract "$1" raw -o - "${manifest}"
}
version="$(manifest_value version)"
manifest_team_id="$(manifest_value signing_team_id)"
artifact_key="artifacts.darwin-${architecture}"
artifact_url="$(manifest_value "${artifact_key}.url")"
expected_sha="$(manifest_value "${artifact_key}.sha256")"
if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]]; then
  printf '发布 manifest 中的版本无效：%s\n' "${version}" >&2
  exit 1
fi
if [[ "${manifest_team_id}" != "${expected_team_id}" ]]; then
  printf '发布 manifest 的 Apple Team ID 与安装器不一致：%s\n' "${manifest_team_id}" >&2
  exit 1
fi
if [[ ! "${expected_sha}" =~ ^[0-9a-fA-F]{64}$ ]]; then
  printf '%s\n' '发布 manifest 中的 SHA256 无效。' >&2
  exit 1
fi
if [[ -z "${extension_id}" ]]; then
  extension_id="$(manifest_value extension_id 2>/dev/null || true)"
fi
if [[ -n "${extension_id}" && ! "${extension_id}" =~ ^[a-p]{32}$ ]]; then
  printf '%s\n' '发布 manifest 中的 Chrome extension id 无效。' >&2
  exit 1
fi

archive="${temporary_root}/agent-ferry.tar.gz"
/usr/bin/curl -fsSL --proto '=https,file' --tlsv1.2 "${artifact_url}" -o "${archive}"
actual_sha="$(/usr/bin/shasum -a 256 "${archive}" | awk '{print $1}')"
if [[ "${actual_sha}" != "${expected_sha}" ]]; then
  printf '发布包 SHA256 不匹配：预期 %s，实际 %s\n' "${expected_sha}" "${actual_sha}" >&2
  exit 1
fi

extracted="${temporary_root}/extracted"
mkdir -p "${extracted}"
/usr/bin/tar -xzf "${archive}" -C "${extracted}"
package_root="${extracted}/agent-ferry-v${version}-darwin-${architecture}"
for binary in aferry agentferryd agentferry-host; do
  binary_path="${package_root}/bin/${binary}"
  if [[ ! -x "${binary_path}" ]]; then
    printf '发布包缺少可执行文件：%s\n' "${binary}" >&2
    exit 1
  fi
  "${codesign_bin}" --verify --strict --verbose=2 "${binary_path}"
  signature_details="$("${codesign_bin}" -d --verbose=4 "${binary_path}" 2>&1)"
  if [[ "${signature_details}" != *"Authority=Developer ID Application:"* ||
        "${signature_details}" != *"TeamIdentifier=${expected_team_id}"* ||
        "${signature_details}" != *"runtime"* ]]; then
    printf '发布包签名身份或 Hardened Runtime 校验失败：%s\n' "${binary}" >&2
    exit 1
  fi
done
if [[ "$("${package_root}/bin/aferry" --version)" != "aferry ${version}" ]]; then
  printf '%s\n' '发布包版本与 manifest 不一致。' >&2
  exit 1
fi

if [[ -e "${current_link}" || -L "${current_link}" ]]; then
  if [[ ! -L "${current_link}" ]]; then
    printf 'current 不是符号链接，拒绝覆盖：%s\n' "${current_link}" >&2
    exit 1
  fi
  previous_current="$(readlink "${current_link}")"
fi
for binary in aferry agentferryd agentferry-host; do
  command_path="${command_root}/${binary}"
  if [[ -e "${command_path}" || -L "${command_path}" ]]; then
    if [[ ! -L "${command_path}" || "$(readlink "${command_path}")" != "${install_root}/current/bin/${binary}" ]]; then
      printf '命令路径已被其他安装占用，拒绝覆盖：%s\n' "${command_path}" >&2
      exit 1
    fi
  fi
done

target_version="${versions_root}/${version}"
if [[ -e "${target_version}" ]]; then
  target_backup="${versions_root}/.${version}.previous.$$"
  mv "${target_version}" "${target_backup}"
fi
mv "${package_root}" "${target_version}"
target_installed="true"
new_aferry="${target_version}/bin/aferry"

plist="${HOME}/Library/LaunchAgents/com.agentferry.daemon.plist"
if [[ -f "${plist}" ]]; then
  old_plist_snapshot="${temporary_root}/previous-launch-agent.plist"
  cp "${plist}" "${old_plist_snapshot}"
  "${new_aferry}" service stop >/dev/null
fi
native_manifest="${HOME}/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.agentferry.host.json"
if [[ -f "${native_manifest}" ]]; then
  old_manifest_snapshot="${temporary_root}/previous-native-host.json"
  cp "${native_manifest}" "${old_manifest_snapshot}"
fi

migration_report="$("${new_aferry}" data migrate --json)"
if printf '%s' "${migration_report}" | grep -q '"state": "migrated"'; then
  data_migrated="true"
fi

next_link="${install_root}/.current.$$"
ln -s "versions/${version}" "${next_link}"
/bin/mv -f -h "${next_link}" "${current_link}"
current_switched="true"
for binary in aferry agentferryd agentferry-host; do
  ln -sfn "${install_root}/current/bin/${binary}" "${command_root}/${binary}"
done

"${install_root}/current/bin/aferry" service install \
  --daemon-path "${install_root}/current/bin/agentferryd" >/dev/null
if [[ -n "${extension_id}" ]]; then
  "${install_root}/current/bin/aferry" native-host register \
    --extension-id "${extension_id}" \
    --host-path "${install_root}/current/bin/agentferry-host"
  native_manifest_touched="true"
fi
"${install_root}/current/bin/aferry" service status --json >/dev/null

install_record="${HOME}/.agent-ferry/install.json"
/usr/bin/plutil -create xml1 "${install_record}"
/usr/bin/plutil -insert version -string "${version}" "${install_record}"
/usr/bin/plutil -insert architecture -string "${architecture}" "${install_record}"
/usr/bin/plutil -insert signing_team_id -string "${expected_team_id}" "${install_record}"
/usr/bin/plutil -insert manifest_url -string "${manifest_url}" "${install_record}"
if [[ -n "${extension_id}" ]]; then
  /usr/bin/plutil -insert extension_id -string "${extension_id}" "${install_record}"
fi
/usr/bin/plutil -convert json -r "${install_record}"
chmod 600 "${install_record}"

committed="true"
rm -rf "${target_backup}"
printf '安装完成：Agent Ferry %s（darwin-%s）\n' "${version}" "${architecture}"
printf '命令目录：%s\n' "${command_root}"
if [[ ":${PATH}:" != *":${command_root}:"* ]]; then
  printf '提示：请把 %s 加入 PATH，然后重新打开终端。\n' "${command_root}"
fi
