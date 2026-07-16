use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use agent_ferry_core::AgentFerryPaths;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

pub const KEYCHAIN_SERVICE: &str = "com.agentferry.hermes";
const KEYCHAIN_REFERENCE_PREFIX: &str = "keychain:com.agentferry.hermes:";
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25_300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesConnection {
    pub id: String,
    pub name: String,
    pub endpoint: HermesEndpoint,
    pub transport: HermesTransport,
    pub credential_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesEndpoint {
    pub base_url: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesTransport {
    Direct,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesConnections {
    pub connections: Vec<HermesConnection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisState {
    Ready,
    CredentialMissing,
    AuthenticationFailed,
    ConnectionFailed,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionDiagnosis {
    pub id: String,
    pub name: String,
    pub state: DiagnosisState,
    pub detail: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
// 字段逐一镜像 Hermes 官方 capability JSON；合并为本地状态机会丢失独立能力组合。
#[allow(clippy::struct_excessive_bools)]
struct HermesFeatures {
    #[serde(default)]
    run_submission: bool,
    #[serde(default)]
    run_status: bool,
    #[serde(default)]
    run_events_sse: bool,
    #[serde(default)]
    run_stop: bool,
    #[serde(default)]
    run_approval_response: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct HermesCapabilities {
    object: String,
    platform: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    features: HermesFeatures,
}

impl HermesConnection {
    /// 构建经过验证的 Direct Hermes Connection。
    ///
    /// # Errors
    ///
    /// 名称、URL 或可选 model 不符合持久配置约束时返回错误。
    pub fn direct(name: &str, base_url: &str, model: Option<String>) -> Result<Self, HermesError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HermesError::InvalidName);
        }
        if model
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(HermesError::InvalidModel);
        }

        let mut base_url = Url::parse(base_url).map_err(HermesError::InvalidUrl)?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(HermesError::UnsafeUrl);
        }
        let normalized_path = base_url.path().trim_end_matches('/').to_owned();
        base_url.set_path(if normalized_path.is_empty() {
            "/"
        } else {
            &normalized_path
        });

        let id = Uuid::new_v4().to_string();
        Ok(Self {
            credential_ref: format!("{KEYCHAIN_REFERENCE_PREFIX}{id}"),
            id,
            name: name.to_owned(),
            endpoint: HermesEndpoint { base_url, model },
            transport: HermesTransport::Direct,
        })
    }

    fn credential_account(&self) -> Result<&str, HermesError> {
        self.credential_ref
            .strip_prefix(KEYCHAIN_REFERENCE_PREFIX)
            .ok_or(HermesError::InvalidCredentialReference)
    }

    fn capabilities_url(&self) -> Result<Url, HermesError> {
        let base = self.endpoint.base_url.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}/v1/capabilities")).map_err(HermesError::InvalidUrl)
    }
}

pub trait CredentialStore: Send + Sync {
    /// 保存或更新凭据值。
    ///
    /// # Errors
    ///
    /// 系统凭据存储不可用时返回错误。
    fn set(&self, reference: &str, secret: &[u8]) -> Result<(), HermesError>;

    /// 读取凭据值；不存在时返回 `None`。
    ///
    /// # Errors
    ///
    /// 系统凭据存储拒绝访问或返回无效数据时返回错误。
    fn get(&self, reference: &str) -> Result<Option<Vec<u8>>, HermesError>;

    /// 删除凭据值；不存在时仍视为成功。
    ///
    /// # Errors
    ///
    /// 系统凭据存储拒绝删除时返回错误。
    fn delete(&self, reference: &str) -> Result<(), HermesError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct KeychainCredentialStore;

impl KeychainCredentialStore {
    fn account(reference: &str) -> Result<&str, HermesError> {
        reference
            .strip_prefix(KEYCHAIN_REFERENCE_PREFIX)
            .ok_or(HermesError::InvalidCredentialReference)
    }
}

impl CredentialStore for KeychainCredentialStore {
    fn set(&self, reference: &str, secret: &[u8]) -> Result<(), HermesError> {
        if secret.is_empty() {
            return Err(HermesError::EmptyCredential);
        }
        let account = Self::account(reference)?;
        security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, secret)
            .map_err(|error| HermesError::CredentialStore(error.to_string()))
    }

    fn get(&self, reference: &str) -> Result<Option<Vec<u8>>, HermesError> {
        let account = Self::account(reference)?;
        let options = security_framework::passwords::PasswordOptions::new_generic_password(
            KEYCHAIN_SERVICE,
            account,
        );
        match security_framework::passwords::generic_password(options) {
            Ok(secret) => Ok(Some(secret)),
            // Security.framework 的 item-not-found 是稳定 OSStatus；只吞掉这一种，
            // Keychain 锁定或 ACL 拒绝必须继续暴露为诊断错误。
            Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(HermesError::CredentialStore(error.to_string())),
        }
    }

    fn delete(&self, reference: &str) -> Result<(), HermesError> {
        let account = Self::account(reference)?;
        match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(HermesError::CredentialStore(error.to_string())),
        }
    }
}

/// 读取不含明文 secret 的 Hermes Connection 配置。
///
/// # Errors
///
/// 配置文件不可读、JSON 无效或包含重复 ID/名称时返回错误。
pub fn load_connections(path: &Path) -> Result<HermesConnections, HermesError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HermesConnections::default());
        }
        Err(error) => return Err(HermesError::Io(error)),
    };
    let connections: HermesConnections = serde_json::from_slice(&bytes)?;
    for (index, connection) in connections.connections.iter().enumerate() {
        if connections.connections[..index]
            .iter()
            .any(|candidate| candidate.id == connection.id || candidate.name == connection.name)
        {
            return Err(HermesError::DuplicateConnection(connection.name.clone()));
        }
        connection.credential_account()?;
    }
    Ok(connections)
}

/// 以原子替换方式保存 Hermes Connection 配置。
///
/// # Errors
///
/// 私有目录创建、序列化、写入或原子替换失败时返回错误。
pub fn save_connections(
    paths: &AgentFerryPaths,
    connections: &HermesConnections,
) -> Result<(), HermesError> {
    paths.ensure_private_config()?;
    let temporary = paths
        .hermes_connections
        .with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(connections)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &paths.hermes_connections)?;
    fs::set_permissions(&paths.hermes_connections, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// 将新 Connection 与凭据作为一个业务操作保存。
///
/// # Errors
///
/// 名称重复、Keychain 写入或配置持久化失败时返回错误；配置失败会回滚新凭据。
pub fn add_connection<S: CredentialStore>(
    paths: &AgentFerryPaths,
    store: &S,
    connection: HermesConnection,
    token: &[u8],
) -> Result<(), HermesError> {
    let mut connections = load_connections(&paths.hermes_connections)?;
    if connections
        .connections
        .iter()
        .any(|candidate| candidate.name == connection.name)
    {
        return Err(HermesError::DuplicateConnection(connection.name));
    }
    store.set(&connection.credential_ref, token)?;
    connections.connections.push(connection.clone());
    if let Err(error) = save_connections(paths, &connections) {
        let _ = store.delete(&connection.credential_ref);
        return Err(error);
    }
    Ok(())
}

/// 删除 Connection 配置及其 Keychain 凭据。
///
/// # Errors
///
/// Connection 不存在、配置保存或 Keychain 删除失败时返回错误；凭据删除失败会恢复配置。
pub fn remove_connection<S: CredentialStore>(
    paths: &AgentFerryPaths,
    store: &S,
    identifier: &str,
) -> Result<HermesConnection, HermesError> {
    let mut connections = load_connections(&paths.hermes_connections)?;
    let index = connections
        .connections
        .iter()
        .position(|connection| connection.id == identifier || connection.name == identifier)
        .ok_or_else(|| HermesError::ConnectionNotFound(identifier.to_owned()))?;
    let connection = connections.connections.remove(index);
    save_connections(paths, &connections)?;
    if let Err(error) = store.delete(&connection.credential_ref) {
        connections.connections.insert(index, connection.clone());
        let _ = save_connections(paths, &connections);
        return Err(error);
    }
    Ok(connection)
}

#[derive(Debug, Clone)]
pub struct HermesClient {
    client: Client,
}

impl HermesClient {
    /// 创建带有界超时且禁止重定向的诊断客户端。
    ///
    /// # Errors
    ///
    /// TLS 或 HTTP client 初始化失败时返回错误。
    pub fn new(timeout: Duration) -> Result<Self, HermesError> {
        let client = Client::builder()
            .timeout(timeout)
            // Bearer Token 不能跟随服务端重定向到另一个 origin；用户应配置最终 URL。
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(HermesError::HttpClient)?;
        Ok(Self { client })
    }

    /// 使用给定凭据发现服务器能力并返回可操作诊断。
    ///
    /// # Errors
    ///
    /// 仅本地 URL 构造等客户端不变量失败时返回错误；服务器状态被归入诊断结果。
    pub async fn diagnose(
        &self,
        connection: &HermesConnection,
        token: &[u8],
    ) -> Result<ConnectionDiagnosis, HermesError> {
        let token = std::str::from_utf8(token).map_err(|_| HermesError::CredentialNotUtf8)?;
        let response = self
            .client
            .get(connection.capabilities_url()?)
            .bearer_auth(token)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return Ok(diagnosis(
                    connection,
                    DiagnosisState::ConnectionFailed,
                    format!("无法连接 Hermes: {error}"),
                    Vec::new(),
                ));
            }
        };
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Ok(diagnosis(
                connection,
                DiagnosisState::AuthenticationFailed,
                "Hermes 拒绝 Bearer Token".to_owned(),
                Vec::new(),
            ));
        }
        if !response.status().is_success() {
            return Ok(diagnosis(
                connection,
                DiagnosisState::ConnectionFailed,
                format!(
                    "Hermes capability discovery 返回 HTTP {}",
                    response.status()
                ),
                Vec::new(),
            ));
        }

        let capabilities: HermesCapabilities = match response.json().await {
            Ok(capabilities) => capabilities,
            Err(error) => {
                return Ok(diagnosis(
                    connection,
                    DiagnosisState::Incompatible,
                    format!("Hermes capabilities 响应无效: {error}"),
                    Vec::new(),
                ));
            }
        };
        if capabilities.object != "hermes.api_server.capabilities"
            || capabilities.platform != "hermes-agent"
            || !capabilities.features.run_submission
            || !capabilities.features.run_status
        {
            return Ok(diagnosis(
                connection,
                DiagnosisState::Incompatible,
                "服务器未声明 Agent Ferry 所需的 run_submission 与 run_status".to_owned(),
                capability_names(&capabilities.features),
            ));
        }

        let model = capabilities
            .model
            .or_else(|| connection.endpoint.model.clone())
            .unwrap_or_else(|| "hermes-agent".to_owned());
        Ok(diagnosis(
            connection,
            DiagnosisState::Ready,
            format!("Hermes {model} capability discovery 通过"),
            capability_names(&capabilities.features),
        ))
    }

    /// 从系统凭据存储读取 token 后诊断连接。
    ///
    /// # Errors
    ///
    /// Keychain 访问或 HTTP client 内部错误时返回错误。
    pub async fn diagnose_with_store<S: CredentialStore>(
        &self,
        connection: &HermesConnection,
        store: &S,
    ) -> Result<ConnectionDiagnosis, HermesError> {
        let Some(token) = store.get(&connection.credential_ref)? else {
            return Ok(diagnosis(
                connection,
                DiagnosisState::CredentialMissing,
                "Keychain 中未找到 Hermes Bearer Token".to_owned(),
                Vec::new(),
            ));
        };
        self.diagnose(connection, &token).await
    }
}

fn diagnosis(
    connection: &HermesConnection,
    state: DiagnosisState,
    detail: String,
    capabilities: Vec<String>,
) -> ConnectionDiagnosis {
    ConnectionDiagnosis {
        id: connection.id.clone(),
        name: connection.name.clone(),
        state,
        detail,
        capabilities,
    }
}

fn capability_names(features: &HermesFeatures) -> Vec<String> {
    let mut capabilities = Vec::new();
    if features.run_submission {
        capabilities.push("run.submit".to_owned());
    }
    if features.run_status {
        capabilities.push("run.status".to_owned());
    }
    if features.run_events_sse {
        capabilities.push("run.events_sse".to_owned());
    }
    if features.run_stop {
        capabilities.push("run.stop".to_owned());
    }
    if features.run_approval_response {
        capabilities.push("run.approval_response".to_owned());
    }
    capabilities
}

#[derive(Debug, thiserror::Error)]
pub enum HermesError {
    #[error("Connection 名称不能为空")]
    InvalidName,
    #[error("model 不能为空字符串")]
    InvalidModel,
    #[error("Hermes URL 无效: {0}")]
    InvalidUrl(url::ParseError),
    #[error("Hermes URL 必须是无内嵌凭据、query 或 fragment 的 http(s) 地址")]
    UnsafeUrl,
    #[error("Hermes credential reference 无效")]
    InvalidCredentialReference,
    #[error("Hermes Bearer Token 不能为空")]
    EmptyCredential,
    #[error("Hermes Bearer Token 不是 UTF-8")]
    CredentialNotUtf8,
    #[error("Connection 已存在: {0}")]
    DuplicateConnection(String),
    #[error("未找到 Hermes Connection: {0}")]
    ConnectionNotFound(String),
    #[error("macOS Keychain 操作失败: {0}")]
    CredentialStore(String),
    #[error("Hermes HTTP client 初始化失败: {0}")]
    HttpClient(reqwest::Error),
    #[error(transparent)]
    Core(#[from] agent_ferry_core::CoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_rejects_embedded_credentials_and_query() {
        assert!(matches!(
            HermesConnection::direct("server", "https://user:secret@example.com", None),
            Err(HermesError::UnsafeUrl)
        ));
        assert!(matches!(
            HermesConnection::direct("server", "https://example.com?token=secret", None),
            Err(HermesError::UnsafeUrl)
        ));
    }

    #[test]
    fn keychain_round_trip_uses_unique_temporary_item() {
        let store = KeychainCredentialStore;
        let connection = HermesConnection::direct("keychain-smoke", "http://127.0.0.1:8642", None)
            .expect("创建 Connection");
        let token = format!("test-token-{}", Uuid::new_v4());
        store
            .set(&connection.credential_ref, token.as_bytes())
            .expect("写入 Keychain");
        assert_eq!(
            store
                .get(&connection.credential_ref)
                .expect("读取 Keychain"),
            Some(token.into_bytes())
        );
        store
            .delete(&connection.credential_ref)
            .expect("删除 Keychain 测试项");
        assert_eq!(
            store
                .get(&connection.credential_ref)
                .expect("复查 Keychain"),
            None
        );
    }
}
