use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use agent_ferry_transport::valid_ssh_host;

const REMOTE_SCRIPT: &str = r#"#!/bin/sh
set -eu

action=$1
container=$2
backup_container="${container}-before-aferry-api"
state_dir="$HOME/.agent-ferry"
env_file="$state_dir/${container}.env"
key_file="$state_dir/${container}.api-key"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

inspect_value() {
  docker inspect "$container" --format "$1"
}

require_value() {
  template=$1
  expected=$2
  detail=$3
  [ "$(inspect_value "$template")" = "$expected" ] || fail "$detail"
}

require_supported_container() {
  command -v docker >/dev/null 2>&1 || fail '远端未安装 Docker'
  docker inspect "$container" >/dev/null 2>&1 || fail '未找到指定 Hermes 容器'

  image=$(inspect_value '{{.Config.Image}}')
  [ "$image" = 'nousresearch/hermes-agent' ] || case "$image" in
    nousresearch/hermes-agent:*) ;;
    sha256:*)
      current_image_id=$(inspect_value '{{.Image}}')
      backup_image=$(docker inspect "$backup_container" --format '{{.Config.Image}}' 2>/dev/null || true)
      backup_image_id=$(docker inspect "$backup_container" --format '{{.Image}}' 2>/dev/null || true)
      case "$backup_image" in
        nousresearch/hermes-agent|nousresearch/hermes-agent:*) ;;
        *) fail '无法验证当前镜像来自官方 nousresearch/hermes-agent' ;;
      esac
      [ "$backup_image_id" = "$current_image_id" ] || fail '当前容器与回滚容器镜像不一致'
      ;;
    *) fail '容器不是官方 nousresearch/hermes-agent 镜像' ;;
  esac
  [ "$(inspect_value '{{json .Config.Cmd}}')" = '["gateway","run"]' ] || fail '容器不是标准 gateway run 启动方式'
  [ "$(inspect_value '{{.HostConfig.NetworkMode}}')" = 'bridge' ] || fail '仅支持 bridge 网络的标准 Docker 部署'
  [ "$(inspect_value '{{.HostConfig.Privileged}}')" = 'false' ] || fail '拒绝修改 privileged 容器'
  [ "$(inspect_value '{{.HostConfig.RestartPolicy.Name}}')" = 'unless-stopped' ] || fail '仅支持 restart=unless-stopped 的标准部署'
  [ "$(inspect_value '{{.State.Running}}')" = 'true' ] || fail 'Hermes 容器当前未运行'

  # 自动重建只接受 Docker 默认资源与安全配置，避免静默丢失用户自定义限制。
  require_value '{{.HostConfig.Memory}}' '0' '检测到自定义内存限制，请人工配置'
  require_value '{{.HostConfig.MemoryReservation}}' '0' '检测到自定义内存预留，请人工配置'
  require_value '{{.HostConfig.MemorySwap}}' '0' '检测到自定义 swap 限制，请人工配置'
  require_value '{{.HostConfig.NanoCpus}}' '0' '检测到自定义 CPU 限制，请人工配置'
  require_value '{{.HostConfig.CpuShares}}' '0' '检测到自定义 CPU shares，请人工配置'
  require_value '{{.HostConfig.CpuPeriod}}' '0' '检测到自定义 CPU period，请人工配置'
  require_value '{{.HostConfig.CpuQuota}}' '0' '检测到自定义 CPU quota，请人工配置'
  require_value '{{.HostConfig.CpusetCpus}}' '' '检测到自定义 CPU 集合，请人工配置'
  require_value '{{.HostConfig.ShmSize}}' '67108864' '检测到自定义 shm-size，请人工配置'
  require_value '{{.HostConfig.ReadonlyRootfs}}' 'false' '检测到只读 rootfs，请人工配置'
  require_value '{{.HostConfig.AutoRemove}}' 'false' '检测到 auto-remove，请人工配置'
  require_value '{{.HostConfig.PublishAllPorts}}' 'false' '检测到 publish-all，请人工配置'
  require_value '{{json .HostConfig.SecurityOpt}}' 'null' '检测到自定义 security-opt，请人工配置'
  require_value '{{json .HostConfig.CapAdd}}' 'null' '检测到额外 Linux capabilities，请人工配置'
  require_value '{{json .HostConfig.CapDrop}}' 'null' '检测到移除 Linux capabilities，请人工配置'
  require_value '{{json .HostConfig.Dns}}' 'null' '检测到自定义 DNS，请人工配置'
  require_value '{{json .HostConfig.Ulimits}}' '[]' '检测到自定义 ulimit，请人工配置'
  require_value '{{json .HostConfig.Devices}}' '[]' '检测到额外设备，请人工配置'
  require_value '{{json .HostConfig.DeviceRequests}}' 'null' '检测到设备请求，请人工配置'
  require_value '{{json .HostConfig.ExtraHosts}}' 'null' '检测到 extra-hosts，请人工配置'
  require_value '{{json .HostConfig.GroupAdd}}' 'null' '检测到附加用户组，请人工配置'
  require_value '{{json .HostConfig.PidsLimit}}' 'null' '检测到 PID 限制，请人工配置'
  require_value '{{json (index .HostConfig "Sysctls")}}' 'null' '检测到自定义 sysctl，请人工配置'
  require_value '{{json (index .HostConfig "Tmpfs")}}' 'null' '检测到 tmpfs，请人工配置'

  image_id=$(inspect_value '{{.Image}}')
  [ "$(inspect_value '{{.HostConfig.LogConfig.Type}}')" = "$(docker info --format '{{.LoggingDriver}}')" ] || fail '检测到自定义日志驱动，请人工配置'
  require_value '{{json .HostConfig.LogConfig.Config}}' '{}' '检测到自定义日志选项，请人工配置'
  [ "$(inspect_value '{{.HostConfig.Runtime}}')" = "$(docker info --format '{{.DefaultRuntime}}')" ] || fail '检测到自定义容器 runtime，请人工配置'
  [ "$(inspect_value '{{json .Config.Labels}}')" = "$(docker image inspect "$image_id" --format '{{json .Config.Labels}}')" ] || fail '检测到自定义容器 labels，请人工配置'
  [ "$(inspect_value '{{json (index .Config "Healthcheck")}}')" = "$(docker image inspect "$image_id" --format '{{json (index .Config "Healthcheck")}}')" ] || fail '检测到自定义 healthcheck，请人工配置'
  [ "$(inspect_value '{{json .Config.User}}')" = "$(docker image inspect "$image_id" --format '{{json .Config.User}}')" ] || fail '检测到自定义容器 user，请人工配置'
  [ "$(inspect_value '{{json .Config.Entrypoint}}')" = "$(docker image inspect "$image_id" --format '{{json .Config.Entrypoint}}')" ] || fail '检测到自定义 entrypoint，请人工配置'
  [ "$(inspect_value '{{json .Config.WorkingDir}}')" = "$(docker image inspect "$image_id" --format '{{json .Config.WorkingDir}}')" ] || fail '检测到自定义 working directory，请人工配置'

  mount_count=$(inspect_value '{{len .Mounts}}')
  [ "$mount_count" = '1' ] || fail '容器包含额外挂载，无法安全自动重建'
  data_type=$(inspect_value '{{range .Mounts}}{{if eq .Destination "/opt/data"}}{{.Type}}{{end}}{{end}}')
  data_source=$(inspect_value '{{range .Mounts}}{{if eq .Destination "/opt/data"}}{{.Source}}{{end}}{{end}}')
  [ "$data_type" = 'bind' ] || fail '/opt/data 不是受支持的 bind mount'
  [ -n "$data_source" ] || fail '未找到 /opt/data 数据目录'

  port_lines=$(docker port "$container" 2>/dev/null || true)
  printf '%s\n' "$port_lines" | grep -q '^9119/tcp -> 0\.0\.0\.0:9119$' || fail '缺少标准 9119 IPv4 端口映射'
  printf '%s\n' "$port_lines" | grep -q '^9119/tcp -> \[::\]:9119$' || fail '缺少标准 9119 IPv6 端口映射'
  unexpected_ports=$(printf '%s\n' "$port_lines" | grep -Ev '^(9119/tcp -> (0\.0\.0\.0:9119|\[::\]:9119)|8642/tcp -> 127\.0\.0\.1:8642)$' || true)
  [ -z "$unexpected_ports" ] || fail '容器包含额外端口映射，无法安全自动重建'

  require_value '{{len .NetworkSettings.Networks}}' '1' '容器连接了额外 Docker network，请人工配置'
  require_value '{{with index .NetworkSettings.Networks "bridge"}}{{json .Aliases}}{{end}}' 'null' '容器包含自定义 network aliases，请人工配置'
}

verify_canonical_config() {
  # 使用相同参数创建一个不启动的探针，完整比较 HostConfig；这比持续枚举 Docker 参数更可靠。
  probe_container="${container}-aferry-probe-$$"
  probe_marker="$$"
  probe_created=false
  cleanup_probe() {
    [ "$probe_created" = 'true' ] || return 0
    marker=$(docker inspect "$probe_container" --format '{{index .Config.Labels "com.agentferry.probe"}}' 2>/dev/null || true)
    if [ "$marker" = "$probe_marker" ]; then
      docker rm -f "$probe_container" >/dev/null 2>&1 || printf '%s\n' '临时配置探针清理失败，请人工检查' >&2
    fi
  }
  trap cleanup_probe EXIT HUP INT TERM
  # 先启用带 label 校验的清理，再创建探针，避免 create 成功与状态记录之间的中断窗口。
  probe_created=true
  if docker port "$container" 8642/tcp >/dev/null 2>&1; then
    docker create --name "$probe_container" --label "com.agentferry.probe=$probe_marker" --restart unless-stopped \
      -v "$data_source:/opt/data" -p 9119:9119 -p 127.0.0.1:8642:8642 \
      "$image_id" gateway run >/dev/null
  else
    docker create --name "$probe_container" --label "com.agentferry.probe=$probe_marker" --restart unless-stopped \
      -v "$data_source:/opt/data" -p 9119:9119 \
      "$image_id" gateway run >/dev/null
  fi
  # Docker 在 run 与 create 路径上会把同一个默认值分别序列化为 null/false；仅规范化这一组等价值。
  existing_host_config=$(inspect_value '{{json .HostConfig}}' | sed 's/"OomKillDisable":null/"OomKillDisable":false/')
  probe_host_config=$(docker inspect "$probe_container" --format '{{json .HostConfig}}' | sed 's/"OomKillDisable":null/"OomKillDisable":false/')
  [ "$existing_host_config" = "$probe_host_config" ] || fail '容器包含无法安全重放的 Docker HostConfig，请人工配置'
  [ "$(inspect_value '{{json (index .Config "StopTimeout")}}')" = "$(docker inspect "$probe_container" --format '{{json (index .Config "StopTimeout")}}')" ] || fail '检测到自定义 stop-timeout，请人工配置'
  [ "$(inspect_value '{{json (index .Config "StopSignal")}}')" = "$(docker inspect "$probe_container" --format '{{json (index .Config "StopSignal")}}')" ] || fail '检测到自定义 stop-signal，请人工配置'
  require_value '{{.Config.Domainname}}' '' '检测到自定义 domainname，请人工配置'
  require_value '{{.Config.AttachStdin}}' 'false' '检测到 attach-stdin，请人工配置'
  require_value '{{.Config.OpenStdin}}' 'false' '检测到 open-stdin，请人工配置'
  require_value '{{.Config.StdinOnce}}' 'false' '检测到 stdin-once，请人工配置'
  require_value '{{.Config.Tty}}' 'false' '检测到 TTY，请人工配置'
  container_id=$(inspect_value '{{.Id}}')
  default_hostname=$(printf '%s' "$container_id" | cut -c1-12)
  [ "$(inspect_value '{{.Config.Hostname}}')" = "$default_hostname" ] || fail '检测到自定义 hostname，请人工配置'
  docker rm -f "$probe_container" >/dev/null || fail '临时配置探针清理失败，未修改 Hermes 主容器'
  probe_created=false
  trap - EXIT HUP INT TERM
}

api_is_ready() {
  [ -s "$key_file" ] || return 1
  published=$(docker port "$container" 8642/tcp 2>/dev/null || true)
  [ "$published" = '127.0.0.1:8642' ] || return 1
  curl -fsS --max-time 5 \
    -H "Authorization: Bearer $(cat "$key_file")" \
    http://127.0.0.1:8642/v1/capabilities >/dev/null 2>&1
}

print_inspection() {
  if api_is_ready; then
    ready=true
  else
    ready=false
    docker inspect "$backup_container" >/dev/null 2>&1 && fail '检测到旧的回滚容器，请先人工确认其状态'
    command -v openssl >/dev/null 2>&1 || fail '远端缺少 openssl，无法生成 API Key'
    command -v curl >/dev/null 2>&1 || fail '远端缺少 curl，无法验证 API'
  fi
  printf '%s\n' \
    'AFERRY_HERMES_INSPECT_V1' \
    "ready=$ready" \
    "container=$container" \
    "backup_container=$backup_container" \
    "image=$image" \
    "data_source=$data_source"
}

rollback_needed=false
rollback() {
  [ "$rollback_needed" = 'true' ] || return 0
  rollback_failed=false
  if docker inspect "$backup_container" >/dev/null 2>&1; then
    if docker inspect "$container" >/dev/null 2>&1; then
      docker rm -f "$container" >/dev/null 2>&1 || rollback_failed=true
    fi
    docker rename "$backup_container" "$container" >/dev/null 2>&1 || rollback_failed=true
  fi
  docker update --restart=unless-stopped "$container" >/dev/null 2>&1 || rollback_failed=true
  docker start "$container" >/dev/null 2>&1 || rollback_failed=true
  running=$(docker inspect "$container" --format '{{.State.Running}}' 2>/dev/null || true)
  if [ "$rollback_failed" = 'false' ] && [ "$running" = 'true' ]; then
    printf '%s\n' 'Hermes 准备失败；旧容器已验证恢复运行' >&2
  else
    printf '%s\n' '紧急：Hermes 自动回滚未完成，请人工检查当前容器和回滚容器' >&2
  fi
}

apply_setup() {
  if api_is_ready; then
    token=$(cat "$key_file")
    printf '%s\n' 'AFERRY_HERMES_SETUP_V1' "token=$token"
    return 0
  fi
  docker inspect "$backup_container" >/dev/null 2>&1 && fail '检测到旧的回滚容器，请先人工确认其状态'
  verify_canonical_config
  mkdir -p "$state_dir"
  chmod 700 "$state_dir"
  umask 077
  docker inspect "$container" --format '{{range .Config.Env}}{{println .}}{{end}}' > "$env_file"
  token=$(openssl rand -hex 32)
  printf '%s\n' \
    'API_SERVER_ENABLED=true' \
    'API_SERVER_HOST=0.0.0.0' \
    'API_SERVER_PORT=8642' \
    "API_SERVER_KEY=$token" >> "$env_file"
  printf '%s\n' "$token" > "$key_file"
  chmod 600 "$env_file" "$key_file"

  image_id=$(inspect_value '{{.Image}}')
  rollback_needed=true
  trap rollback EXIT HUP INT TERM
  docker stop "$container" >/dev/null
  docker update --restart=no "$container" >/dev/null
  docker rename "$container" "$backup_container"

  docker run -d \
    --name "$container" \
    --restart unless-stopped \
    --env-file "$env_file" \
    -v "$data_source:/opt/data" \
    -p 9119:9119 \
    -p 127.0.0.1:8642:8642 \
    "$image_id" gateway run >/dev/null

  attempts=0
  until api_is_ready; do
    attempts=$((attempts + 1))
    [ "$attempts" -lt 30 ] || fail 'Hermes API 未在 30 秒内就绪，将验证并恢复旧容器'
    sleep 1
  done
  rollback_needed=false
  trap - EXIT HUP INT TERM
  printf '%s\n' 'AFERRY_HERMES_SETUP_V1' "token=$token"
}

require_supported_container
case "$action" in
  inspect) print_inspection ;;
  apply) apply_setup ;;
  *) fail '未知操作' ;;
esac
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HermesPreflight {
    pub ready: bool,
    pub container: String,
    pub backup_container: String,
    pub image: String,
    pub data_source: String,
}

pub struct PreparedHermes {
    token: String,
}

impl PreparedHermes {
    pub fn into_token(self) -> String {
        self.token
    }
}

impl fmt::Debug for PreparedHermes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedHermes")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

pub struct SshHermesSetup {
    program: PathBuf,
    timeout: Duration,
}

impl SshHermesSetup {
    pub fn system() -> Self {
        Self {
            program: PathBuf::from("/usr/bin/ssh"),
            timeout: Duration::from_secs(90),
        }
    }

    pub fn inspect(
        &self,
        ssh_host: &str,
        container: &str,
    ) -> Result<HermesPreflight, HermesSetupError> {
        validate_inputs(ssh_host, container)?;
        let output = self.execute(ssh_host, "inspect", container)?;
        let preflight = parse_preflight(&output)?;
        if preflight.container != container
            || preflight.backup_container != format!("{container}-before-aferry-api")
        {
            return Err(HermesSetupError::InvalidProtocol);
        }
        Ok(preflight)
    }

    pub fn apply(
        &self,
        ssh_host: &str,
        container: &str,
    ) -> Result<PreparedHermes, HermesSetupError> {
        validate_inputs(ssh_host, container)?;
        let output = self.execute(ssh_host, "apply", container)?;
        parse_setup(&output)
    }

    fn execute(
        &self,
        ssh_host: &str,
        action: &str,
        container: &str,
    ) -> Result<String, HermesSetupError> {
        let mut child = Command::new(&self.program)
            .args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "--",
                ssh_host,
                "sh",
                "-s",
                "--",
                action,
                container,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| HermesSetupError::Spawn(self.program.clone(), error))?;
        child
            .stdin
            .take()
            .ok_or(HermesSetupError::MissingStdin)?
            .write_all(REMOTE_SCRIPT.as_bytes())?;
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HermesSetupError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(HermesSetupError::RemoteFailed {
                code: output.status.code(),
                detail: sanitize_remote_error(&output.stderr),
            });
        }
        String::from_utf8(output.stdout).map_err(|_| HermesSetupError::InvalidProtocol)
    }
}

fn validate_inputs(ssh_host: &str, container: &str) -> Result<(), HermesSetupError> {
    if !valid_ssh_host(ssh_host) {
        return Err(HermesSetupError::InvalidSshHost);
    }
    if container.is_empty()
        || container.len() > 128
        || !container
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(HermesSetupError::InvalidContainer);
    }
    Ok(())
}

fn parse_preflight(output: &str) -> Result<HermesPreflight, HermesSetupError> {
    let mut lines = output.lines();
    if lines.next() != Some("AFERRY_HERMES_INSPECT_V1") {
        return Err(HermesSetupError::InvalidProtocol);
    }
    let ready = required_field(&mut lines, "ready=")?;
    let container = required_field(&mut lines, "container=")?;
    let backup_container = required_field(&mut lines, "backup_container=")?;
    let image = required_field(&mut lines, "image=")?;
    let data_source = required_field(&mut lines, "data_source=")?;
    if lines.next().is_some()
        || [&container, &backup_container, &image, &data_source]
            .iter()
            .any(|value| value.chars().any(char::is_control))
        || !data_source.starts_with('/')
        || data_source.contains(':')
    {
        return Err(HermesSetupError::InvalidProtocol);
    }
    Ok(HermesPreflight {
        ready: match ready.as_str() {
            "true" => true,
            "false" => false,
            _ => return Err(HermesSetupError::InvalidProtocol),
        },
        container,
        backup_container,
        image,
        data_source,
    })
}

fn parse_setup(output: &str) -> Result<PreparedHermes, HermesSetupError> {
    let mut lines = output.lines();
    if lines.next() != Some("AFERRY_HERMES_SETUP_V1") {
        return Err(HermesSetupError::InvalidProtocol);
    }
    let token = required_field(&mut lines, "token=")?;
    if lines.next().is_some()
        || token.len() < 8
        || token.len() > 16 * 1024
        || token.chars().any(char::is_whitespace)
    {
        return Err(HermesSetupError::InvalidProtocol);
    }
    Ok(PreparedHermes { token })
}

fn required_field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Result<String, HermesSetupError> {
    let value = lines
        .next()
        .and_then(|line| line.strip_prefix(prefix))
        .filter(|value| !value.is_empty())
        .ok_or(HermesSetupError::InvalidProtocol)?;
    Ok(value.to_owned())
}

fn sanitize_remote_error(stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr);
    let line = detail.lines().last().unwrap_or("远端命令失败").trim();
    if line.is_empty()
        || line.len() > 512
        || line.contains("API_SERVER_KEY=")
        || line.chars().any(char::is_control)
    {
        "远端命令失败；未输出可能包含凭据的详情".to_owned()
    } else {
        line.to_owned()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HermesSetupError {
    #[error("SSH 目标无效；请使用 user@host 或单一 ~/.ssh/config Host，不能包含空白或以 - 开头")]
    InvalidSshHost,
    #[error("Docker 容器名称无效")]
    InvalidContainer,
    #[error("无法启动系统 SSH {0}: {1}")]
    Spawn(PathBuf, std::io::Error),
    #[error("无法向 SSH 写入远端准备脚本")]
    MissingStdin,
    #[error("SSH 或远端 Docker 操作失败（code={code:?}）：{detail}")]
    RemoteFailed { code: Option<i32>, detail: String },
    #[error("SSH 远端准备超过 90 秒")]
    Timeout,
    #[error("远端准备响应无效；为避免泄漏凭据，已隐藏原始响应")]
    InvalidProtocol,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Stdio;

    use uuid::Uuid;

    const FAKE_DOCKER: &str = r#"#!/bin/sh
set -eu
state="$HOME/fake-docker"
command=$1
shift
printf '%s %s\n' "$command" "$*" >> "$state/calls.log"
case "$command" in
  inspect)
    name=$1
    shift
    case "$name" in
      hermes) marker="$state/current" ;;
      hermes-before-aferry-api) marker="$state/backup" ;;
      hermes-aferry-probe-*) marker="$state/probe" ;;
      *) exit 1 ;;
    esac
    [ -f "$marker" ] || exit 1
    [ "$#" -gt 0 ] || exit 0
    [ "$1" = '--format' ]
    template=$2
    case "$template" in
      '{{.Config.Image}}') printf '%s\n' 'nousresearch/hermes-agent' ;;
      '{{json .Config.Cmd}}') printf '%s\n' '["gateway","run"]' ;;
      '{{.HostConfig.NetworkMode}}') printf '%s\n' 'bridge' ;;
      '{{.HostConfig.Privileged}}') printf '%s\n' 'false' ;;
      '{{.HostConfig.RestartPolicy.Name}}') printf '%s\n' 'unless-stopped' ;;
      '{{.State.Running}}') printf '%s\n' 'true' ;;
      '{{len .Mounts}}') printf '%s\n' '1' ;;
      *Mounts*Type*) printf '%s\n' 'bind' ;;
      *Mounts*Source*) printf '%s\n' "$HOME/.hermes" ;;
      '{{.HostConfig.Memory}}') if [ -f "$state/custom-memory" ]; then printf '%s\n' '1073741824'; else printf '%s\n' '0'; fi ;;
      '{{.HostConfig.MemoryReservation}}'|'{{.HostConfig.MemorySwap}}'|'{{.HostConfig.NanoCpus}}'|'{{.HostConfig.CpuShares}}'|'{{.HostConfig.CpuPeriod}}'|'{{.HostConfig.CpuQuota}}') printf '%s\n' '0' ;;
      '{{.HostConfig.CpusetCpus}}') printf '\n' ;;
      '{{.HostConfig.ShmSize}}') printf '%s\n' '67108864' ;;
      '{{.HostConfig.ReadonlyRootfs}}'|'{{.HostConfig.AutoRemove}}'|'{{.HostConfig.PublishAllPorts}}') printf '%s\n' 'false' ;;
      '{{json .HostConfig.Ulimits}}'|'{{json .HostConfig.Devices}}') printf '%s\n' '[]' ;;
      '{{json .HostConfig.LogConfig.Config}}') printf '%s\n' '{}' ;;
      '{{.HostConfig.LogConfig.Type}}') printf '%s\n' 'json-file' ;;
      '{{.HostConfig.Runtime}}') printf '%s\n' 'runc' ;;
      '{{json .Config.Labels}}') printf '%s\n' '{"org.opencontainers.image.revision":"fake"}' ;;
      '{{json (index .Config "Healthcheck")}}') printf '%s\n' 'null' ;;
      '{{json .Config.User}}') printf '%s\n' '"root"' ;;
      '{{json .Config.Entrypoint}}') printf '%s\n' '["/init","/opt/hermes/docker/main-wrapper.sh"]' ;;
      '{{json .Config.WorkingDir}}') printf '%s\n' '"/opt/hermes"' ;;
      '{{len .NetworkSettings.Networks}}') printf '%s\n' '1' ;;
      '{{with index .NetworkSettings.Networks "bridge"}}{{json .Aliases}}{{end}}') printf '%s\n' 'null' ;;
      '{{json .HostConfig}}') printf '%s\n' '{"canonical":true}' ;;
      '{{json (index .Config "StopTimeout")}}'|'{{json (index .Config "StopSignal")}}') printf '%s\n' 'null' ;;
      '{{.Config.Domainname}}') printf '\n' ;;
      '{{.Config.AttachStdin}}'|'{{.Config.OpenStdin}}'|'{{.Config.StdinOnce}}'|'{{.Config.Tty}}') printf '%s\n' 'false' ;;
      '{{.Id}}') printf '%s\n' 'abcdef1234567890' ;;
      '{{.Config.Hostname}}') printf '%s\n' 'abcdef123456' ;;
      '{{json .HostConfig.SecurityOpt}}'|'{{json .HostConfig.CapAdd}}'|'{{json .HostConfig.CapDrop}}'|'{{json .HostConfig.Dns}}'|'{{json .HostConfig.DeviceRequests}}'|'{{json .HostConfig.ExtraHosts}}'|'{{json .HostConfig.GroupAdd}}'|'{{json .HostConfig.PidsLimit}}'|'{{json (index .HostConfig "Sysctls")}}'|'{{json (index .HostConfig "Tmpfs")}}') printf '%s\n' 'null' ;;
      '{{.Image}}') printf '%s\n' 'sha256:fake-hermes-image' ;;
      *'.Config.Env'*) printf '%s\n' 'HERMES_DASHBOARD=true' ;;
      *) exit 2 ;;
    esac
    ;;
  port)
    name=$1
    shift
    [ "$name" = 'hermes' ] && [ -f "$state/current" ] || exit 1
    mode=$(cat "$state/mode")
    if [ "$#" -gt 0 ]; then
      [ "$1" = '8642/tcp' ] && [ "$mode" = 'ready' ] || exit 1
      printf '%s\n' '127.0.0.1:8642'
    else
      printf '%s\n' '9119/tcp -> 0.0.0.0:9119' '9119/tcp -> [::]:9119'
      [ "$mode" != 'ready' ] || printf '%s\n' '8642/tcp -> 127.0.0.1:8642'
    fi
    ;;
  stop|update)
    ;;
  rename)
    old=$1
    new=$2
    if [ "$old" = 'hermes' ] && [ "$new" = 'hermes-before-aferry-api' ]; then
      mv "$state/current" "$state/backup"
    elif [ "$old" = 'hermes-before-aferry-api' ] && [ "$new" = 'hermes' ]; then
      mv "$state/backup" "$state/current"
      printf '%s\n' 'initial' > "$state/mode"
    else
      exit 3
    fi
    ;;
  run)
    [ ! -f "$state/fail-run" ] || exit 9
    : > "$state/current"
    printf '%s\n' 'ready' > "$state/mode"
    printf '%s\n' 'fake-container-id'
    ;;
  create)
    : > "$state/probe"
    printf '%s\n' 'fake-probe-id'
    ;;
  rm)
    target=''
    for argument in "$@"; do target=$argument; done
    case "$target" in
      hermes) rm -f "$state/current" ;;
      hermes-aferry-probe-*) rm -f "$state/probe" ;;
      *) exit 7 ;;
    esac
    ;;
  start)
    : > "$state/current"
    ;;
  info)
    template=$2
    case "$template" in
      '{{.LoggingDriver}}') printf '%s\n' 'json-file' ;;
      '{{.DefaultRuntime}}') printf '%s\n' 'runc' ;;
      *) exit 5 ;;
    esac
    ;;
  image)
    [ "$1" = 'inspect' ]
    template=$4
    case "$template" in
      '{{json .Config.Labels}}') printf '%s\n' '{"org.opencontainers.image.revision":"fake"}' ;;
      '{{json (index .Config "Healthcheck")}}') printf '%s\n' 'null' ;;
      '{{json .Config.User}}') printf '%s\n' '"root"' ;;
      '{{json .Config.Entrypoint}}') printf '%s\n' '["/init","/opt/hermes/docker/main-wrapper.sh"]' ;;
      '{{json .Config.WorkingDir}}') printf '%s\n' '"/opt/hermes"' ;;
      *) exit 6 ;;
    esac
    ;;
  *) exit 4 ;;
esac
"#;

    struct FakeRemote {
        root: PathBuf,
    }

    impl FakeRemote {
        fn new(curl_succeeds: bool) -> Self {
            let root = PathBuf::from(format!("/tmp/af-hs-{}", Uuid::new_v4().simple()));
            let bin = root.join("bin");
            let state = root.join("fake-docker");
            fs::create_dir_all(root.join(".hermes")).expect("创建 fake 数据目录");
            fs::create_dir_all(&bin).expect("创建 fake bin");
            fs::create_dir_all(&state).expect("创建 fake Docker 状态");
            fs::write(state.join("current"), "").expect("创建当前容器标记");
            fs::write(state.join("mode"), "initial\n").expect("创建模式标记");
            Self::write_executable(&bin.join("docker"), FAKE_DOCKER);
            Self::write_executable(
                &bin.join("openssl"),
                "#!/bin/sh\nprintf '%s\\n' '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'\n",
            );
            let curl_exit = i32::from(!curl_succeeds);
            Self::write_executable(&bin.join("curl"), &format!("#!/bin/sh\nexit {curl_exit}\n"));
            Self::write_executable(&bin.join("sleep"), "#!/bin/sh\nexit 0\n");
            Self { root }
        }

        fn write_executable(path: &std::path::Path, content: &str) {
            fs::write(path, content).expect("写入 fake 命令");
            let mut permissions = fs::metadata(path).expect("读取 fake 命令").permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).expect("设置 fake 命令权限");
        }

        fn run(&self, action: &str) -> std::process::Output {
            let mut child = Command::new("/bin/sh")
                .args(["-s", "--", action, "hermes"])
                .env("HOME", &self.root)
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("启动远端脚本");
            child
                .stdin
                .take()
                .expect("获取远端脚本 stdin")
                .write_all(REMOTE_SCRIPT.as_bytes())
                .expect("写入远端脚本");
            child.wait_with_output().expect("等待远端脚本")
        }

        fn enable_failure(&self, marker: &str) {
            fs::write(self.root.join("fake-docker").join(marker), "").expect("设置 fake 故障标记");
        }
    }

    impl Drop for FakeRemote {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn validates_ssh_and_container_inputs() {
        assert!(validate_inputs("ktoon-hermes", "hermes").is_ok());
        assert!(validate_inputs("root@ktoon.site", "hermes-prod_1").is_ok());
        assert!(validate_inputs("-oProxyCommand=bad", "hermes").is_err());
        assert!(validate_inputs("valid", "hermes; reboot").is_err());
    }

    #[test]
    fn parses_strict_preflight_protocol() {
        let parsed = parse_preflight(
            "AFERRY_HERMES_INSPECT_V1\nready=false\ncontainer=hermes\nbackup_container=hermes-before-aferry-api\nimage=nousresearch/hermes-agent\ndata_source=/root/.hermes\n",
        )
        .expect("解析预检");
        assert!(!parsed.ready);
        assert_eq!(parsed.data_source, "/root/.hermes");
        assert!(parse_preflight("noise\nready=false\n").is_err());
    }

    #[test]
    fn prepared_token_is_redacted_and_protocol_is_strict() {
        let secret = "0123456789abcdef";
        let prepared = parse_setup(&format!("AFERRY_HERMES_SETUP_V1\ntoken={secret}\n"))
            .expect("解析准备结果");
        assert!(!format!("{prepared:?}").contains(secret));
        assert!(parse_setup("AFERRY_HERMES_SETUP_V1\ntoken=short\n").is_err());
        assert!(parse_setup("AFERRY_HERMES_SETUP_V1\ntoken=secret value\n").is_err());
    }

    #[test]
    fn remote_errors_hide_environment_lines() {
        let error = sanitize_remote_error(b"failure\nAPI_SERVER_KEY=must-not-leak\n");
        assert!(!error.contains("must-not-leak"));
        assert!(REMOTE_SCRIPT.contains("127.0.0.1:8642:8642"));
        assert!(REMOTE_SCRIPT.contains("rollback"));
    }

    #[test]
    fn remote_script_prepares_and_reuses_standard_docker_gateway() {
        let remote = FakeRemote::new(true);
        let inspection = remote.run("inspect");
        assert!(
            inspection.status.success(),
            "预检失败: {}; calls={}",
            String::from_utf8_lossy(&inspection.stderr),
            fs::read_to_string(remote.root.join("fake-docker/calls.log"))
                .unwrap_or_else(|_| "<无>".to_owned())
        );
        let preflight = parse_preflight(&String::from_utf8(inspection.stdout).expect("预检输出"))
            .expect("解析预检");
        assert!(!preflight.ready);
        let inspect_calls =
            fs::read_to_string(remote.root.join("fake-docker/calls.log")).expect("读取预检调用");
        assert!(
            !inspect_calls
                .lines()
                .any(|line| line.starts_with("create "))
        );

        let application = remote.run("apply");
        assert!(
            application.status.success(),
            "准备失败: {}",
            String::from_utf8_lossy(&application.stderr)
        );
        let output = String::from_utf8(application.stdout).expect("准备输出");
        let prepared = parse_setup(&output).expect("解析准备结果");
        assert!(!format!("{prepared:?}").contains("0123456789abcdef"));
        assert!(remote.root.join("fake-docker/backup").is_file());
        assert!(remote.root.join("fake-docker/current").is_file());

        let repeated = remote.run("inspect");
        let repeated = parse_preflight(&String::from_utf8(repeated.stdout).expect("重复预检输出"))
            .expect("解析重复预检");
        assert!(repeated.ready);
    }

    #[test]
    fn remote_script_rolls_back_when_capability_check_never_succeeds() {
        let remote = FakeRemote::new(false);
        let application = remote.run("apply");
        assert!(!application.status.success());
        assert!(remote.root.join("fake-docker/current").is_file());
        assert!(!remote.root.join("fake-docker/backup").exists());
        assert_eq!(
            fs::read_to_string(remote.root.join("fake-docker/mode")).expect("读取回滚模式"),
            "initial\n"
        );
        assert!(!String::from_utf8_lossy(&application.stderr).contains("0123456789abcdef"));
    }

    #[test]
    fn remote_script_rejects_custom_resources_before_any_change() {
        let remote = FakeRemote::new(true);
        remote.enable_failure("custom-memory");
        let inspection = remote.run("inspect");
        assert!(!inspection.status.success());
        assert!(String::from_utf8_lossy(&inspection.stderr).contains("内存限制"));
        assert!(remote.root.join("fake-docker/current").is_file());
        assert!(!remote.root.join("fake-docker/backup").exists());
    }

    #[test]
    fn remote_script_verifies_rollback_when_docker_run_fails_before_creation() {
        let remote = FakeRemote::new(true);
        remote.enable_failure("fail-run");
        let application = remote.run("apply");
        assert!(!application.status.success());
        assert!(remote.root.join("fake-docker/current").is_file());
        assert!(!remote.root.join("fake-docker/backup").exists());
        let stderr = String::from_utf8_lossy(&application.stderr);
        assert!(stderr.contains("旧容器已验证恢复运行"));
        assert!(!stderr.contains("紧急"));
    }
}
