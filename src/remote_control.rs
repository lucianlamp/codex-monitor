use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use base64::Engine;
#[cfg(not(windows))]
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(not(windows))]
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
    MaybeTlsStream, WebSocketStream,
};

const DEFAULT_PROD_API_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_DEV_API_BASE_URL: &str = "http://localhost:8000/api";
#[cfg(target_os = "macos")]
const DEFAULT_DEVICE_KEY_MODULE_PATH: &str =
    "/Applications/Codex.app/Contents/Resources/native/remote-control-device-key.node";
#[cfg(not(windows))]
const PAIR_CLIENT_PATH: &str = "/wham/remote/control/client/pair";
#[cfg(not(windows))]
const BACKEND_CLIENTS_PATH: &str = "/wham/remote/control/clients";
#[cfg(not(windows))]
const REMOTE_CLIENT_WS_PATH: &str = "/codex/remote/control/client";
#[cfg(not(windows))]
const REMOTE_CLIENT_REFRESH_START_PATH: &str = "/codex/remote/control/client/refresh/start";
#[cfg(not(windows))]
const REMOTE_CLIENT_REFRESH_FINISH_PATH: &str = "/codex/remote/control/client/refresh/finish";
#[cfg(not(windows))]
const REMOTE_CONTROL_WEBSOCKET_SCOPE: &str = "remote_control_controller_websocket";
#[cfg(not(windows))]
const REMOTE_CONTROL_PROTOCOL_VERSION: &str = "3";
const ENROLLMENTS_KEY: &str = "electron-remote-control-client-enrollments";

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlStatus {
    pub status: String,
    #[serde(rename = "serverName")]
    pub server_name: String,
    #[serde(rename = "installationId")]
    pub installation_id: String,
    #[serde(rename = "environmentId")]
    pub environment_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlPairingStart {
    #[serde(rename = "environmentId")]
    pub environment_id: String,
    #[serde(rename = "pairingCode")]
    pub pairing_code: String,
    #[serde(rename = "manualPairingCode")]
    pub manual_pairing_code: Option<String>,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlPairingStatus {
    pub claimed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlClient {
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub platform: Option<String>,
    #[serde(rename = "deviceType")]
    pub device_type: Option<String>,
    #[serde(rename = "deviceModel")]
    pub device_model: Option<String>,
    #[serde(rename = "osVersion")]
    pub os_version: Option<String>,
    #[serde(rename = "appVersion")]
    pub app_version: Option<String>,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RemoteControlClientsList {
    data: Vec<RemoteControlClient>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlBackendClient {
    #[serde(rename = "client_id")]
    pub client_id: String,
    #[serde(rename = "display_name")]
    pub display_name: Option<String>,
    #[serde(rename = "device_type")]
    pub device_type: Option<String>,
    pub platform: Option<String>,
    #[serde(rename = "device_model")]
    pub device_model: Option<String>,
    #[serde(rename = "last_seen_at")]
    pub last_seen_at: Option<String>,
    #[serde(rename = "enrollment_status")]
    pub enrollment_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteControlBackendClientsPage {
    items: Vec<RemoteControlBackendClient>,
    cursor: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RemoteControlDeviceKeyStatus {
    Available,
    Unavailable,
    Mismatch,
    Unsupported,
}

impl RemoteControlDeviceKeyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Mismatch => "mismatch",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteControlDeviceKeyCheck {
    pub client_id: String,
    pub key_id: String,
    pub status: RemoteControlDeviceKeyStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChatGptAuthIdentity {
    pub account_id: Option<String>,
    pub account_user_id: Option<String>,
    pub auth_user_id: Option<String>,
}

impl ChatGptAuthIdentity {
    pub fn account_user_id_candidates(&self) -> Vec<&str> {
        let mut candidates = Vec::new();
        if let Some(account_user_id) = self.account_user_id.as_deref() {
            candidates.push(account_user_id);
        }
        if let Some(auth_user_id) = self.auth_user_id.as_deref() {
            if !candidates.contains(&auth_user_id) {
                candidates.push(auth_user_id);
            }
        }
        candidates
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteControlClientPairClaim {
    pub api_base_url: String,
    pub access_token: String,
    pub account_id: Option<String>,
    pub client_id: String,
    pub manual_pairing_code: String,
    pub user_agent: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct RemoteControlClientEnrollmentRecord {
    #[serde(rename = "accountUserId")]
    pub account_user_id: String,
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "keyId")]
    pub key_id: String,
    pub algorithm: String,
    #[serde(rename = "protectionClass")]
    pub protection_class: String,
    #[serde(rename = "publicKeySpkiDerBase64")]
    pub public_key_spki_der_base64: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteControlClientConnectOptions {
    pub api_base_url: String,
    pub websocket_url: Option<String>,
    pub access_token: String,
    pub account_id: Option<String>,
    pub enrollment: RemoteControlClientEnrollmentRecord,
    pub device_key_module_path: PathBuf,
    pub user_agent: String,
    pub timeout: Duration,
    pub max_messages: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteControlClientConnectResult {
    pub client_id: String,
    pub token_expires_at: i64,
    pub scopes: Vec<String>,
    pub proof_algorithm: String,
    pub messages: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexAuthTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthTokens {
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteControlClientEnrollment {
    #[serde(rename = "accountUserId")]
    account_user_id: Option<String>,
    #[serde(rename = "clientId")]
    client_id: Option<String>,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct RemoteControlRefreshStartResponse {
    account_user_id: String,
    client_id: String,
    device_key_challenge: RemoteControlEnrollmentChallenge,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct RemoteControlRefreshFinishResponse {
    account_user_id: String,
    client_id: String,
    remote_control_token: String,
    expires_at: String,
    scopes: Vec<String>,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct RemoteControlEnrollmentChallenge {
    challenge_id: String,
    challenge_token: String,
    nonce: String,
    purpose: String,
    audience: String,
    account_user_id: String,
    client_id: String,
    target_origin: String,
    target_path: String,
    device_identity_hash: Option<String>,
    challenge_expires_at: Value,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct RemoteControlWebsocketChallenge {
    #[serde(rename = "type")]
    envelope_type: String,
    nonce: String,
    purpose: String,
    audience: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "targetOrigin")]
    target_origin: String,
    #[serde(rename = "targetPath")]
    target_path: String,
    #[serde(rename = "accountUserId")]
    account_user_id: String,
    #[serde(rename = "clientId")]
    client_id: String,
    #[serde(rename = "tokenSha256Base64url")]
    token_sha256_base64url: String,
    #[serde(rename = "tokenExpiresAt")]
    token_expires_at: i64,
    scopes: Vec<String>,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct DeviceKeySignature {
    #[serde(rename = "signatureDerBase64")]
    signature_der_base64: String,
    #[serde(rename = "signedPayloadBase64")]
    signed_payload_base64: String,
    algorithm: String,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct DeviceKeyPublic {
    #[serde(rename = "keyId")]
    key_id: String,
    #[serde(rename = "publicKeySpkiDerBase64")]
    public_key_spki_der_base64: String,
    algorithm: String,
    #[serde(rename = "protectionClass")]
    protection_class: String,
}

pub fn parse_status(value: &Value) -> anyhow::Result<RemoteControlStatus> {
    Ok(serde_json::from_value(value.clone())?)
}

pub fn parse_pairing_start(value: &Value) -> anyhow::Result<RemoteControlPairingStart> {
    Ok(serde_json::from_value(value.clone())?)
}

pub fn parse_pairing_status(value: &Value) -> anyhow::Result<RemoteControlPairingStatus> {
    Ok(serde_json::from_value(value.clone())?)
}

pub fn parse_clients(value: &Value) -> anyhow::Result<Vec<RemoteControlClient>> {
    let parsed: RemoteControlClientsList = serde_json::from_value(value.clone())?;
    Ok(parsed.data)
}

pub fn default_auth_file_path() -> PathBuf {
    codex_home_path().join("auth.json")
}

pub fn default_global_state_file_path() -> PathBuf {
    codex_home_path().join(".codex-global-state.json")
}

pub fn default_device_key_module_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(DEFAULT_DEVICE_KEY_MODULE_PATH)
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("remote-control-device-key.node")
    }
}

pub fn default_api_base_url() -> String {
    if let Ok(value) = std::env::var("CODEX_API_BASE_URL") {
        let trimmed = value.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if std::env::var("CODEX_API_ENDPOINT")
        .map(|value| value.trim().eq_ignore_ascii_case("localhost"))
        .unwrap_or(false)
    {
        return DEFAULT_DEV_API_BASE_URL.to_string();
    }
    DEFAULT_PROD_API_BASE_URL.to_string()
}

pub fn read_chatgpt_access_token(path: &Path) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read auth file {}", path.display()))?;
    let parsed: CodexAuthFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse auth file {}", path.display()))?;
    parsed
        .tokens
        .and_then(|tokens| tokens.access_token)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "auth file {} does not contain tokens.access_token",
                path.display()
            )
        })
}

pub fn parse_chatgpt_auth_identity(access_token: &str) -> ChatGptAuthIdentity {
    let auth = jwt_payload(access_token)
        .and_then(|payload| payload.get("https://api.openai.com/auth").cloned())
        .unwrap_or(Value::Null);
    ChatGptAuthIdentity {
        account_id: string_field(&auth, "chatgpt_account_id")
            .or_else(|| string_field(&auth, "account_id")),
        account_user_id: string_field(&auth, "chatgpt_account_user_id")
            .or_else(|| string_field(&auth, "account_user_id")),
        auth_user_id: string_field(&auth, "user_id"),
    }
}

pub fn resolve_enrolled_client_id_from_file(
    path: &Path,
    account_user_id: Option<&str>,
) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read global state file {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse global state file {}", path.display()))?;
    resolve_enrolled_client_id(&value, account_user_id).with_context(|| {
        format!(
            "failed to resolve remote-control client enrollment from {}",
            path.display()
        )
    })
}

pub fn resolve_enrolled_client_id_from_file_for_identity(
    path: &Path,
    identity: &ChatGptAuthIdentity,
) -> anyhow::Result<String> {
    let candidates = identity.account_user_id_candidates();
    if candidates.is_empty() {
        return resolve_enrolled_client_id_from_file(path, None);
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read global state file {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse global state file {}", path.display()))?;
    resolve_enrolled_client_id_for_candidates(&value, &candidates).with_context(|| {
        format!(
            "failed to resolve remote-control client enrollment from {}",
            path.display()
        )
    })
}

pub fn resolve_enrolled_client_record_from_file_for_identity(
    path: &Path,
    identity: &ChatGptAuthIdentity,
) -> anyhow::Result<RemoteControlClientEnrollmentRecord> {
    resolve_enrolled_client_record_from_file_for_identity_and_client_id(path, identity, None)
}

pub fn resolve_enrolled_client_record_from_file_for_identity_and_client_id(
    path: &Path,
    identity: &ChatGptAuthIdentity,
    client_id: Option<&str>,
) -> anyhow::Result<RemoteControlClientEnrollmentRecord> {
    let candidates = identity.account_user_id_candidates();
    if candidates.is_empty() {
        bail!("no ChatGPT user id found in access token");
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read global state file {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse global state file {}", path.display()))?;
    if let Some(client_id) = client_id {
        return resolve_enrolled_client_record_for_candidates_and_client_id(
            &value,
            &candidates,
            client_id,
        )
        .with_context(|| {
            format!(
                "failed to resolve remote-control client enrollment from {}",
                path.display()
            )
        });
    }
    resolve_enrolled_client_record_for_candidates(&value, &candidates).with_context(|| {
        format!(
            "failed to resolve remote-control client enrollment from {}",
            path.display()
        )
    })
}

pub fn resolve_enrolled_client_id(
    global_state: &Value,
    account_user_id: Option<&str>,
) -> anyhow::Result<String> {
    let candidates = account_user_id.into_iter().collect::<Vec<_>>();
    resolve_enrolled_client_id_for_candidates(global_state, &candidates)
}

pub fn resolve_enrolled_client_id_for_candidates(
    global_state: &Value,
    account_user_ids: &[&str],
) -> anyhow::Result<String> {
    let enrollments = global_state
        .get(ENROLLMENTS_KEY)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("global state has no {ENROLLMENTS_KEY} object"))?;

    let mut matches = Vec::new();
    for value in enrollments.values() {
        let Ok(enrollment) = serde_json::from_value::<RemoteControlClientEnrollment>(value.clone())
        else {
            continue;
        };
        let Some(client_id) = enrollment
            .client_id
            .filter(|client_id| !client_id.is_empty())
        else {
            continue;
        };
        if !account_user_ids.is_empty()
            && enrollment
                .account_user_id
                .as_deref()
                .is_none_or(|account_user_id| !account_user_ids.contains(&account_user_id))
        {
            continue;
        }
        matches.push(client_id);
    }

    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [client_id] => Ok(client_id.clone()),
        [] if !account_user_ids.is_empty() => {
            bail!("no enrolled remote-control client matched the current ChatGPT user ids")
        }
        [] => bail!("no enrolled remote-control client found"),
        many => bail!(
            "multiple enrolled remote-control clients found; pass --client-id explicitly: {}",
            many.join(", ")
        ),
    }
}

pub fn resolve_enrolled_client_record_for_candidates(
    global_state: &Value,
    account_user_ids: &[&str],
) -> anyhow::Result<RemoteControlClientEnrollmentRecord> {
    resolve_enrolled_client_record_for_candidates_inner(global_state, account_user_ids, None)
}

pub fn resolve_enrolled_client_record_for_candidates_and_client_id(
    global_state: &Value,
    account_user_ids: &[&str],
    client_id: &str,
) -> anyhow::Result<RemoteControlClientEnrollmentRecord> {
    resolve_enrolled_client_record_for_candidates_inner(
        global_state,
        account_user_ids,
        Some(client_id),
    )
}

fn resolve_enrolled_client_record_for_candidates_inner(
    global_state: &Value,
    account_user_ids: &[&str],
    client_id: Option<&str>,
) -> anyhow::Result<RemoteControlClientEnrollmentRecord> {
    let enrollments = global_state
        .get(ENROLLMENTS_KEY)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("global state has no {ENROLLMENTS_KEY} object"))?;

    let mut matches = Vec::new();
    for value in enrollments.values() {
        let Ok(enrollment) =
            serde_json::from_value::<RemoteControlClientEnrollmentRecord>(value.clone())
        else {
            continue;
        };
        if !account_user_ids.is_empty()
            && !account_user_ids.contains(&enrollment.account_user_id.as_str())
        {
            continue;
        }
        if client_id.is_some_and(|client_id| enrollment.client_id != client_id) {
            continue;
        }
        matches.push(enrollment);
    }

    matches.sort_by(|left, right| left.client_id.cmp(&right.client_id));
    matches.dedup_by(|left, right| left.client_id == right.client_id);
    match matches.as_slice() {
        [enrollment] => Ok(enrollment.clone()),
        [] if client_id.is_some() => bail!(
            "no enrolled remote-control client with device key matched client id {}",
            client_id.unwrap_or_default()
        ),
        [] => bail!(
            "no enrolled remote-control client with device key matched the current ChatGPT user ids"
        ),
        many => bail!(
            "multiple enrolled remote-control clients found; pass --client-id explicitly: {}",
            many.iter()
                .map(|enrollment| enrollment.client_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(not(windows))]
pub async fn claim_remote_control_client_pairing(
    claim: &RemoteControlClientPairClaim,
) -> anyhow::Result<Value> {
    let url = backend_url(&claim.api_base_url, PAIR_CLIENT_PATH)?;
    let mut request = reqwest::Client::new()
        .post(url)
        .bearer_auth(&claim.access_token)
        .header("originator", "Codex Desktop")
        .header("user-agent", &claim.user_agent)
        .json(&serde_json::json!({
            "client_id": claim.client_id,
            "manual_pairing_code": claim.manual_pairing_code,
        }));

    if let Some(account_id) = claim.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!(
            "remote-control client pair failed ({}): {}",
            status.as_u16(),
            response_error_detail(&body)
        );
    }
    if body.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(&body).with_context(|| "remote-control client pair response was not JSON")
}

#[cfg(not(windows))]
pub async fn list_remote_control_backend_clients(
    api_base_url: &str,
    access_token: &str,
    account_id: Option<&str>,
    user_agent: &str,
) -> anyhow::Result<Vec<RemoteControlBackendClient>> {
    let headers = remote_control_auth_headers(access_token, account_id, user_agent);
    let mut cursor = None::<String>;
    let mut clients = Vec::new();
    loop {
        let mut url = backend_url(api_base_url, BACKEND_CLIENTS_PATH)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "100");
            if let Some(cursor) = cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
        }

        let mut request = reqwest::Client::new().get(url);
        for (name, value) in &headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            bail!(
                "remote-control request {} failed ({}): {}",
                BACKEND_CLIENTS_PATH,
                status.as_u16(),
                response_error_detail(&body)
            );
        }
        let page: RemoteControlBackendClientsPage =
            serde_json::from_str(&body).with_context(|| {
                format!(
                    "remote-control response for {} was not valid JSON",
                    BACKEND_CLIENTS_PATH.trim_start_matches('/')
                )
            })?;
        clients.extend(
            page.items
                .into_iter()
                .filter(|client| client.enrollment_status.as_deref() != Some("pending_enrollment")),
        );
        cursor = page.cursor.filter(|cursor| !cursor.trim().is_empty());
        if cursor.is_none() {
            break;
        }
    }
    Ok(clients)
}

#[cfg(windows)]
pub async fn list_remote_control_backend_clients(
    _api_base_url: &str,
    _access_token: &str,
    _account_id: Option<&str>,
    _user_agent: &str,
) -> anyhow::Result<Vec<RemoteControlBackendClient>> {
    bail!("remote backend clients are not available in Windows builds yet")
}

#[cfg(not(windows))]
pub async fn connect_remote_control_client(
    options: &RemoteControlClientConnectOptions,
) -> anyhow::Result<RemoteControlClientConnectResult> {
    validate_device_key_record(&options.device_key_module_path, &options.enrollment).await?;
    let refresh = refresh_remote_control_client_session(options).await?;
    let token_expires_at = parse_rfc3339_unix_seconds(&refresh.expires_at)?;
    validate_remote_control_token_response(
        &refresh,
        &options.enrollment,
        token_expires_at,
        &[REMOTE_CONTROL_WEBSOCKET_SCOPE],
    )?;

    let websocket_url = match options.websocket_url.as_deref() {
        Some(websocket_url) => websocket_url.to_string(),
        None => websocket_url(&options.api_base_url, REMOTE_CLIENT_WS_PATH)?,
    };
    let token_header = format!("Bearer {}", refresh.remote_control_token);
    let mut headers = remote_control_auth_headers(
        &options.access_token,
        options.account_id.as_deref(),
        &options.user_agent,
    );
    headers.push((
        "x-codex-client-session-token".to_string(),
        token_header.clone(),
    ));
    headers.push((
        "x-codex-client-id".to_string(),
        options.enrollment.client_id.clone(),
    ));
    headers.push((
        "x-codex-protocol-version".to_string(),
        REMOTE_CONTROL_PROTOCOL_VERSION.to_string(),
    ));

    let request = websocket_request(&websocket_url, &headers)?;
    let connect = tokio::time::timeout(options.timeout, connect_async(request))
        .await
        .with_context(|| {
            format!(
                "timed out connecting remote-control websocket after {} ms",
                options.timeout.as_millis()
            )
        })??;
    let (mut websocket, _) = connect;

    let challenge_text = tokio::time::timeout(options.timeout, next_text_message(&mut websocket))
        .await
        .with_context(|| {
            format!(
                "timed out waiting for remote-control device-key challenge after {} ms",
                options.timeout.as_millis()
            )
        })??;
    let challenge: RemoteControlWebsocketChallenge = serde_json::from_str(&challenge_text)
        .with_context(|| "remote-control websocket challenge was not valid JSON")?;
    validate_websocket_challenge(
        &challenge,
        &websocket_url,
        &refresh.remote_control_token,
        token_expires_at,
        &refresh.scopes,
        &options.enrollment,
    )?;

    let proof_payload_json =
        websocket_device_key_payload_json(&challenge, token_expires_at, &refresh.scopes)?;
    let proof = sign_device_key_payload_json(
        &options.device_key_module_path,
        &options.enrollment.key_id,
        &proof_payload_json,
    )
    .await?;
    let proof_algorithm = proof.algorithm.clone();
    websocket
        .send(Message::Text(
            serde_json::json!({
                "type": "device_key_proof",
                "keyId": options.enrollment.key_id,
                "signatureDerBase64": &proof.signature_der_base64,
                "signedPayloadBase64": &proof.signed_payload_base64,
                "algorithm": &proof.algorithm,
            })
            .to_string()
            .into(),
        ))
        .await?;

    let mut messages = Vec::new();
    while messages.len() < options.max_messages {
        let message =
            match tokio::time::timeout(options.timeout, next_text_message(&mut websocket)).await {
                Ok(result) => result?,
                Err(_) => break,
            };
        match serde_json::from_str::<Value>(&message) {
            Ok(value) => messages.push(value),
            Err(_) => messages.push(Value::String(message)),
        }
    }

    let _ = websocket.close(None).await;
    Ok(RemoteControlClientConnectResult {
        client_id: options.enrollment.client_id.clone(),
        token_expires_at,
        scopes: refresh.scopes,
        proof_algorithm,
        messages,
    })
}

#[cfg(windows)]
pub async fn claim_remote_control_client_pairing(
    _claim: &RemoteControlClientPairClaim,
) -> anyhow::Result<Value> {
    bail!("remote claim is not available in Windows builds yet")
}

#[cfg(windows)]
pub async fn connect_remote_control_client(
    _options: &RemoteControlClientConnectOptions,
) -> anyhow::Result<RemoteControlClientConnectResult> {
    bail!("remote connect is not available in Windows builds yet")
}

#[cfg(not(windows))]
pub async fn check_device_key_record(
    module_path: &Path,
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> RemoteControlDeviceKeyCheck {
    match get_device_key_public(module_path, &enrollment.key_id).await {
        Ok(public)
            if public.key_id == enrollment.key_id
                && public.public_key_spki_der_base64 == enrollment.public_key_spki_der_base64
                && public.algorithm == enrollment.algorithm
                && public.protection_class == enrollment.protection_class =>
        {
            RemoteControlDeviceKeyCheck {
                client_id: enrollment.client_id.clone(),
                key_id: enrollment.key_id.clone(),
                status: RemoteControlDeviceKeyStatus::Available,
                detail: None,
            }
        }
        Ok(_) => RemoteControlDeviceKeyCheck {
            client_id: enrollment.client_id.clone(),
            key_id: enrollment.key_id.clone(),
            status: RemoteControlDeviceKeyStatus::Mismatch,
            detail: Some("device key public material does not match global state".to_string()),
        },
        Err(error) => RemoteControlDeviceKeyCheck {
            client_id: enrollment.client_id.clone(),
            key_id: enrollment.key_id.clone(),
            status: RemoteControlDeviceKeyStatus::Unavailable,
            detail: Some(error.to_string()),
        },
    }
}

#[cfg(windows)]
pub async fn check_device_key_record(
    _module_path: &Path,
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> RemoteControlDeviceKeyCheck {
    RemoteControlDeviceKeyCheck {
        client_id: enrollment.client_id.clone(),
        key_id: enrollment.key_id.clone(),
        status: RemoteControlDeviceKeyStatus::Unsupported,
        detail: Some("device-key checks are not available in Windows builds yet".to_string()),
    }
}

#[cfg(not(windows))]
async fn refresh_remote_control_client_session(
    options: &RemoteControlClientConnectOptions,
) -> anyhow::Result<RemoteControlRefreshFinishResponse> {
    let headers = remote_control_auth_headers(
        &options.access_token,
        options.account_id.as_deref(),
        &options.user_agent,
    );
    let start: RemoteControlRefreshStartResponse = post_backend_json(
        &options.api_base_url,
        REMOTE_CLIENT_REFRESH_START_PATH,
        &headers,
        serde_json::json!({ "client_id": &options.enrollment.client_id }),
    )
    .await?;
    validate_refresh_start_response(&start, &options.enrollment)?;

    let target = backend_target(&options.api_base_url, REMOTE_CLIENT_REFRESH_FINISH_PATH)?;
    validate_enrollment_challenge(&start.device_key_challenge, &options.enrollment, &target)?;
    let identity_hash = device_identity_sha256_base64url(&options.enrollment);
    let proof_payload_json =
        enrollment_device_key_payload_json(&start.device_key_challenge, &identity_hash)?;
    let proof = sign_device_key_payload_json(
        &options.device_key_module_path,
        &options.enrollment.key_id,
        &proof_payload_json,
    )
    .await?;

    post_backend_json(
        &options.api_base_url,
        REMOTE_CLIENT_REFRESH_FINISH_PATH,
        &headers,
        serde_json::json!({
            "client_id": &options.enrollment.client_id,
            "device_key_proof": {
                "challenge_token": start.device_key_challenge.challenge_token,
                "key_id": &options.enrollment.key_id,
                "signature_der_base64": proof.signature_der_base64,
                "signed_payload_base64": proof.signed_payload_base64,
                "algorithm": proof.algorithm,
            }
        }),
    )
    .await
}

#[cfg(not(windows))]
async fn post_backend_json<T: for<'de> Deserialize<'de>>(
    api_base_url: &str,
    path: &str,
    headers: &[(String, String)],
    body: Value,
) -> anyhow::Result<T> {
    let url = backend_url(api_base_url, path)?;
    let mut request = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string());
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!(
            "remote-control request {} failed ({}): {}",
            path,
            status.as_u16(),
            response_error_detail(&body)
        );
    }
    serde_json::from_str(&body).with_context(|| {
        format!(
            "remote-control response for {} was not valid JSON",
            path.trim_start_matches('/')
        )
    })
}

#[cfg(not(windows))]
fn validate_refresh_start_response(
    response: &RemoteControlRefreshStartResponse,
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> anyhow::Result<()> {
    if response.client_id != enrollment.client_id
        || response.account_user_id != enrollment.account_user_id
    {
        bail!("remote-control refresh challenge does not match local enrollment");
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_remote_control_token_response(
    response: &RemoteControlRefreshFinishResponse,
    enrollment: &RemoteControlClientEnrollmentRecord,
    token_expires_at: i64,
    expected_scopes: &[&str],
) -> anyhow::Result<()> {
    if response.client_id != enrollment.client_id
        || response.account_user_id != enrollment.account_user_id
    {
        bail!("remote-control token response does not match local enrollment");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    if token_expires_at <= now {
        bail!("remote-control token response has invalid expiration");
    }
    if response.scopes.len() != expected_scopes.len()
        || !response
            .scopes
            .iter()
            .zip(expected_scopes)
            .all(|(actual, expected)| actual == expected)
    {
        bail!("remote-control token response has unexpected scopes");
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_enrollment_challenge(
    challenge: &RemoteControlEnrollmentChallenge,
    enrollment: &RemoteControlClientEnrollmentRecord,
    target: &RemoteControlTarget,
) -> anyhow::Result<()> {
    if challenge.purpose != "remote_control_client_enrollment"
        || challenge.audience != "remote_control_client_enrollment"
        || challenge.account_user_id != enrollment.account_user_id
        || challenge.client_id != enrollment.client_id
        || challenge.target_origin != target.origin
        || challenge.target_path != target.path
    {
        bail!("remote-control enrollment challenge does not match local enrollment");
    }
    let expected_hash = device_identity_sha256_base64url(enrollment);
    match challenge.device_identity_hash.as_deref() {
        Some(actual_hash) if actual_hash == expected_hash => Ok(()),
        Some(_) => {
            bail!("remote-control enrollment challenge does not match local device identity")
        }
        None => bail!("remote-control enrollment challenge is missing device identity hash"),
    }
}

#[cfg(not(windows))]
fn validate_websocket_challenge(
    challenge: &RemoteControlWebsocketChallenge,
    websocket_url: &str,
    remote_control_token: &str,
    token_expires_at: i64,
    scopes: &[String],
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> anyhow::Result<()> {
    if challenge.envelope_type != "device_key_challenge"
        || challenge.purpose != "remote_control_client_websocket"
        || challenge.audience != "remote_control_client_websocket"
        || challenge.account_user_id != enrollment.account_user_id
        || challenge.client_id != enrollment.client_id
    {
        bail!("remote-control websocket device-key challenge does not match local enrollment");
    }
    let target = websocket_target(websocket_url)?;
    if challenge.target_origin != target.origin || challenge.target_path != target.path {
        bail!("remote-control websocket device-key challenge target does not match websocket URL");
    }
    if challenge.token_sha256_base64url != sha256_base64url(remote_control_token.as_bytes()) {
        bail!(
            "remote-control websocket device-key challenge token hash does not match session token"
        );
    }
    if challenge.token_expires_at != token_expires_at || challenge.scopes != scopes {
        bail!("remote-control websocket device-key challenge token metadata does not match enrollment");
    }
    Ok(())
}

#[cfg(not(windows))]
async fn sign_device_key_payload_json(
    module_path: &Path,
    key_id: &str,
    payload_json: &str,
) -> anyhow::Result<DeviceKeySignature> {
    let script = r#"
const native = require(process.argv[1]);
const keyId = process.argv[2];
const payload = JSON.parse(process.argv[3]);
const domain = "codex-device-key-sign-payload/v1";
function normalize(payload) {
  switch (payload.type) {
    case "remoteControlClientConnection":
      return {
        accountUserId: payload.accountUserId,
        audience: payload.audience,
        clientId: payload.clientId,
        nonce: payload.nonce,
        scopes: payload.scopes,
        sessionId: payload.sessionId,
        targetOrigin: payload.targetOrigin,
        targetPath: payload.targetPath,
        tokenExpiresAt: payload.tokenExpiresAt,
        tokenSha256Base64url: payload.tokenSha256Base64url,
        type: payload.type
      };
    case "remoteControlClientEnrollment":
      return {
        accountUserId: payload.accountUserId,
        audience: payload.audience,
        challengeExpiresAt: payload.challengeExpiresAt,
        challengeId: payload.challengeId,
        clientId: payload.clientId,
        deviceIdentitySha256Base64url: payload.deviceIdentitySha256Base64url,
        nonce: payload.nonce,
        targetOrigin: payload.targetOrigin,
        targetPath: payload.targetPath,
        type: payload.type
      };
    default:
      throw new Error(`Unsupported device-key payload type: ${payload.type}`);
  }
}
const signedPayload = Buffer.from(JSON.stringify({ domain, payload: normalize(payload) }), "utf8");
Promise.resolve(native.signDeviceKey(keyId, signedPayload))
  .then((result) => {
    process.stdout.write(JSON.stringify({
      ...result,
      signedPayloadBase64: signedPayload.toString("base64")
    }));
  })
  .catch((error) => {
    process.stderr.write(error && error.stack ? error.stack : String(error));
    process.exit(1);
  });
"#;
    let output = tokio::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(module_path)
        .arg(key_id)
        .arg(payload_json)
        .output()
        .await
        .with_context(|| "failed to run node device-key signer")?;
    if !output.status.success() {
        bail!(
            "device-key signer failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| "device-key signer output was not valid JSON")
}

#[cfg(not(windows))]
async fn validate_device_key_record(
    module_path: &Path,
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> anyhow::Result<()> {
    let public = get_device_key_public(module_path, &enrollment.key_id)
        .await
        .with_context(|| {
            format!(
                "remote-control enrollment device key is not available for client {}",
                enrollment.client_id
            )
        })?;
    if public.key_id != enrollment.key_id
        || public.public_key_spki_der_base64 != enrollment.public_key_spki_der_base64
        || public.algorithm != enrollment.algorithm
        || public.protection_class != enrollment.protection_class
    {
        bail!(
            "remote-control enrollment device key does not match global state for client {}",
            enrollment.client_id
        );
    }
    Ok(())
}

#[cfg(not(windows))]
async fn get_device_key_public(
    module_path: &Path,
    key_id: &str,
) -> anyhow::Result<DeviceKeyPublic> {
    let script = r#"
const native = require(process.argv[1]);
const keyId = process.argv[2];
Promise.resolve(native.getDeviceKeyPublic(keyId))
  .then((result) => {
    process.stdout.write(JSON.stringify(result));
  })
  .catch((error) => {
    process.stderr.write(error && error.stack ? error.stack : String(error));
    process.exit(1);
  });
"#;
    let output = tokio::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(module_path)
        .arg(key_id)
        .output()
        .await
        .with_context(|| "failed to run node device-key public-key reader")?;
    if !output.status.success() {
        bail!(
            "device-key public-key reader failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| "device-key public-key reader output was not valid JSON")
}

#[cfg(not(windows))]
async fn next_text_message(
    websocket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> anyhow::Result<String> {
    while let Some(message) = websocket.next().await {
        match message? {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Binary(bytes) => return Ok(String::from_utf8(bytes.to_vec())?),
            Message::Close(frame) => bail!("remote-control websocket closed: {:?}", frame),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
    bail!("remote-control websocket closed")
}

#[cfg(not(windows))]
fn remote_control_auth_headers(
    access_token: &str,
    account_id: Option<&str>,
    user_agent: &str,
) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {}", access_token),
        ),
        ("originator".to_string(), "Codex Desktop".to_string()),
        ("User-Agent".to_string(), user_agent.to_string()),
    ];
    if let Some(account_id) = account_id {
        headers.push(("ChatGPT-Account-Id".to_string(), account_id.to_string()));
    }
    headers
}

#[cfg(not(windows))]
fn websocket_request(
    websocket_url: &str,
    headers: &[(String, String)],
) -> anyhow::Result<http::Request<()>> {
    let mut request = websocket_url
        .into_client_request()
        .with_context(|| format!("failed to build websocket request for {websocket_url}"))?;
    for (name, value) in headers {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid websocket header name: {name}"))?;
        let value = http::HeaderValue::from_str(value)
            .with_context(|| format!("invalid websocket header value for {name}"))?;
        request.headers_mut().insert(name, value);
    }
    Ok(request)
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Eq, PartialEq)]
struct RemoteControlTarget {
    origin: String,
    path: String,
}

#[cfg(not(windows))]
fn backend_target(api_base_url: &str, path: &str) -> anyhow::Result<RemoteControlTarget> {
    let url = backend_url(api_base_url, path)?;
    Ok(RemoteControlTarget {
        origin: url_origin(&url)?,
        path: url.path().to_string(),
    })
}

#[cfg(not(windows))]
fn websocket_target(websocket_url: &str) -> anyhow::Result<RemoteControlTarget> {
    let url = url::Url::parse(websocket_url)?;
    let origin_scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        other => bail!("unsupported remote-control websocket scheme: {other}"),
    };
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("remote-control websocket URL has no host"))?;
    let origin = match url.port() {
        Some(port) => format!("{origin_scheme}://{host}:{port}"),
        None => format!("{origin_scheme}://{host}"),
    };
    Ok(RemoteControlTarget {
        origin,
        path: url.path().to_string(),
    })
}

#[cfg(not(windows))]
fn websocket_url(api_base_url: &str, path: &str) -> anyhow::Result<String> {
    let mut url = backend_url(api_base_url, path)?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => bail!("unsupported remote-control API URL scheme: {other}"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("failed to set websocket URL scheme"))?;
    Ok(url.to_string())
}

#[cfg(not(windows))]
fn url_origin(url: &url::Url) -> anyhow::Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("remote-control API URL has no host"))?;
    Ok(match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    })
}

pub fn device_identity_sha256_base64url(
    enrollment: &RemoteControlClientEnrollmentRecord,
) -> String {
    let identity_json = ordered_json_object(&[
        ("algorithm", Value::String(enrollment.algorithm.clone())),
        ("keyId", Value::String(enrollment.key_id.clone())),
        (
            "protectionClass",
            Value::String(enrollment.protection_class.clone()),
        ),
        (
            "publicKeySpkiDerBase64",
            Value::String(enrollment.public_key_spki_der_base64.clone()),
        ),
    ])
    .expect("device identity fields are JSON-serializable");
    sha256_base64url(identity_json.as_bytes())
}

#[cfg(not(windows))]
fn enrollment_device_key_payload_json(
    challenge: &RemoteControlEnrollmentChallenge,
    device_identity_hash: &str,
) -> anyhow::Result<String> {
    ordered_json_object(&[
        (
            "type",
            Value::String("remoteControlClientEnrollment".to_string()),
        ),
        ("nonce", Value::String(challenge.nonce.clone())),
        ("audience", Value::String(challenge.audience.clone())),
        ("challengeId", Value::String(challenge.challenge_id.clone())),
        (
            "targetOrigin",
            Value::String(challenge.target_origin.clone()),
        ),
        ("targetPath", Value::String(challenge.target_path.clone())),
        (
            "accountUserId",
            Value::String(challenge.account_user_id.clone()),
        ),
        ("clientId", Value::String(challenge.client_id.clone())),
        (
            "deviceIdentitySha256Base64url",
            Value::String(device_identity_hash.to_string()),
        ),
        ("challengeExpiresAt", challenge.challenge_expires_at.clone()),
    ])
}

#[cfg(not(windows))]
fn websocket_device_key_payload_json(
    challenge: &RemoteControlWebsocketChallenge,
    token_expires_at: i64,
    scopes: &[String],
) -> anyhow::Result<String> {
    ordered_json_object(&[
        (
            "type",
            Value::String("remoteControlClientConnection".to_string()),
        ),
        ("nonce", Value::String(challenge.nonce.clone())),
        ("audience", Value::String(challenge.audience.clone())),
        ("sessionId", Value::String(challenge.session_id.clone())),
        (
            "targetOrigin",
            Value::String(challenge.target_origin.clone()),
        ),
        ("targetPath", Value::String(challenge.target_path.clone())),
        (
            "accountUserId",
            Value::String(challenge.account_user_id.clone()),
        ),
        ("clientId", Value::String(challenge.client_id.clone())),
        (
            "tokenSha256Base64url",
            Value::String(challenge.token_sha256_base64url.clone()),
        ),
        (
            "tokenExpiresAt",
            Value::Number(serde_json::Number::from(token_expires_at)),
        ),
        (
            "scopes",
            Value::Array(scopes.iter().cloned().map(Value::String).collect()),
        ),
    ])
}

fn ordered_json_object(fields: &[(&str, Value)]) -> anyhow::Result<String> {
    let mut parts = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        let key = serde_json::to_string(key)?;
        let value = serde_json::to_string(value)?;
        parts.push(format!("{key}:{value}"));
    }
    Ok(format!("{{{}}}", parts.join(",")))
}

fn sha256_base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

#[cfg(not(windows))]
fn parse_rfc3339_unix_seconds(value: &str) -> anyhow::Result<i64> {
    Ok(chrono::DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("failed to parse remote-control token expires_at: {value}"))?
        .timestamp())
}

pub fn default_user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "Macintosh; Intel Mac OS X",
        "linux" => "X11; Linux",
        "windows" => "Windows NT 10.0",
        other => other,
    };
    format!(
        "Codex Desktop/{} ({}; {})",
        crate::VERSION,
        os,
        std::env::consts::ARCH
    )
}

fn codex_home_path() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

#[cfg(not(windows))]
fn backend_url(api_base_url: &str, path: &str) -> anyhow::Result<url::Url> {
    let raw = format!(
        "{}/{}",
        api_base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    url::Url::parse(&raw).with_context(|| format!("invalid API URL: {raw}"))
}

#[cfg(not(windows))]
fn response_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            string_field(&value, "detail")
                .or_else(|| string_field(&value, "message"))
                .or_else(|| string_field(&value, "error"))
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "empty response body".to_string()
            } else {
                trimmed.to_string()
            }
        })
}

fn jwt_payload(access_token: &str) -> Option<Value> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    #[test]
    fn parses_remote_control_status() {
        let parsed = parse_status(&json!({
            "status": "connected",
            "serverName": "mac.local",
            "installationId": "install-1",
            "environmentId": "env-1"
        }))
        .unwrap();

        assert_eq!(parsed.status, "connected");
        assert_eq!(parsed.environment_id.as_deref(), Some("env-1"));
    }

    #[test]
    fn parses_remote_control_clients() {
        let parsed = parse_clients(&json!({
            "data": [
                {
                    "clientId": "client-1",
                    "displayName": "Phone",
                    "platform": "ios",
                    "lastSeenAt": 1781840000000i64
                }
            ],
            "nextCursor": null
        }))
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].client_id, "client-1");
        assert_eq!(parsed[0].display_name.as_deref(), Some("Phone"));
    }

    #[test]
    fn parses_chatgpt_account_identity_from_jwt_payload() {
        let payload = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1",
                "chatgpt_account_user_id": "user-1"
            }
        });
        let token = format!(
            "hdr.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        );

        let parsed = parse_chatgpt_auth_identity(&token);

        assert_eq!(parsed.account_id.as_deref(), Some("acct-1"));
        assert_eq!(parsed.account_user_id.as_deref(), Some("user-1"));
    }

    #[test]
    fn includes_legacy_auth_user_id_as_enrollment_candidate() {
        let payload = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1",
                "chatgpt_account_user_id": "account-user-1",
                "user_id": "auth-user-1"
            }
        });
        let token = format!(
            "hdr.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        );

        let parsed = parse_chatgpt_auth_identity(&token);

        assert_eq!(
            parsed.account_user_id_candidates(),
            vec!["account-user-1", "auth-user-1"]
        );
    }

    #[test]
    fn resolves_enrolled_remote_control_client_for_current_account() {
        let client_id = resolve_enrolled_client_id(
            &json!({
                "electron-remote-control-client-enrollments": {
                    "older": {
                        "accountUserId": "other-user",
                        "clientId": "cli-other"
                    },
                    "current": {
                        "accountUserId": "user-1",
                        "clientId": "cli-current"
                    }
                }
            }),
            Some("user-1"),
        )
        .unwrap();

        assert_eq!(client_id, "cli-current");
    }

    #[test]
    fn resolves_enrolled_remote_control_client_for_any_current_user_id_candidate() {
        let identity = ChatGptAuthIdentity {
            account_id: Some("acct-1".to_string()),
            account_user_id: Some("account-user-1".to_string()),
            auth_user_id: Some("auth-user-1".to_string()),
        };
        let client_id = resolve_enrolled_client_id_for_candidates(
            &json!({
                "electron-remote-control-client-enrollments": {
                    "current": {
                        "accountUserId": "auth-user-1",
                        "clientId": "cli-current"
                    }
                }
            }),
            &identity.account_user_id_candidates(),
        )
        .unwrap();

        assert_eq!(client_id, "cli-current");
    }

    #[test]
    fn resolves_enrolled_remote_control_client_record_with_device_key() {
        let record = resolve_enrolled_client_record_for_candidates(
            &json!({
                "electron-remote-control-client-enrollments": {
                    "current": {
                        "accountUserId": "auth-user-1",
                        "clientId": "cli-current",
                        "keyId": "key-1",
                        "algorithm": "ES256",
                        "protectionClass": "allow_os_protected_nonextractable",
                        "publicKeySpkiDerBase64": "public-key"
                    }
                }
            }),
            &["account-user-1", "auth-user-1"],
        )
        .unwrap();

        assert_eq!(record.client_id, "cli-current");
        assert_eq!(record.key_id, "key-1");
    }

    #[test]
    fn hashes_device_identity_with_codex_app_field_order() {
        let enrollment = RemoteControlClientEnrollmentRecord {
            account_user_id: "user-1".to_string(),
            client_id: "cli-1".to_string(),
            key_id: "key-1".to_string(),
            algorithm: "ES256".to_string(),
            protection_class: "allow_os_protected_nonextractable".to_string(),
            public_key_spki_der_base64: "public-key".to_string(),
        };
        let expected_json = r#"{"algorithm":"ES256","keyId":"key-1","protectionClass":"allow_os_protected_nonextractable","publicKeySpkiDerBase64":"public-key"}"#;

        assert_eq!(
            device_identity_sha256_base64url(&enrollment),
            sha256_base64url(expected_json.as_bytes())
        );
    }

    #[test]
    fn rejects_ambiguous_enrolled_remote_control_clients() {
        let error = resolve_enrolled_client_id(
            &json!({
                "electron-remote-control-client-enrollments": {
                    "first": { "clientId": "cli-a" },
                    "second": { "clientId": "cli-b" }
                }
            }),
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("multiple enrolled"));
    }
}
