//! 领域模型与 IPC 数据结构（serde camelCase，与前端 types.ts 对齐）。

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{ApiError, ApiResult};

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    Password,
    Key,
    Agent,
}

impl AuthType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Key => "key",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "password" => Some(Self::Password),
            "key" => Some(Self::Key),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

// ---------------- 分组 ----------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub sort_order: i64,
    pub profile_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

// ---------------- 连接配置 ----------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInput {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub name: String,
    pub group_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub agent_identity: Option<String>,
    pub term: Option<String>,
    pub charset: Option<String>,
    pub keepalive_secs: Option<i64>,
    pub connect_timeout_secs: Option<i64>,
    pub jump_profile_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub color: Option<String>,
    pub notes: Option<String>,
    /// 只在创建/更新时写入系统钥匙串；读取时永远不返回。
    pub secret: Option<SecretInput>,
}

impl ProfileInput {
    pub fn validate(&self) -> ApiResult<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::bad_request("连接名称需为 1–64 个字符"));
        }
        if self.host.trim().is_empty() || self.host.chars().count() > 255 {
            return Err(ApiError::bad_request("主机地址不能为空且不超过 255 字符"));
        }
        if self.port == 0 {
            return Err(ApiError::bad_request("端口号必须大于 0"));
        }
        if self.username.trim().is_empty() || self.username.chars().count() > 64 {
            return Err(ApiError::bad_request("用户名需为 1–64 个字符"));
        }
        if self.auth_type == AuthType::Key {
            let path = self.key_path.as_deref().unwrap_or("").trim();
            if path.is_empty() {
                return Err(ApiError::bad_request("私钥认证必须填写私钥路径"));
            }
        }
        if let Some(keepalive) = self.keepalive_secs {
            if !(0..=600).contains(&keepalive) {
                return Err(ApiError::bad_request("keepalive 需在 0–600 秒之间"));
            }
        }
        if let Some(timeout) = self.connect_timeout_secs {
            if !(5..=120).contains(&timeout) {
                return Err(ApiError::bad_request("连接超时需在 5–120 秒之间"));
            }
        }
        if let Some(secret) = &self.secret {
            if secret.kind != "password" && secret.kind != "passphrase" {
                return Err(ApiError::bad_request(
                    "secret.kind 只能是 password 或 passphrase",
                ));
            }
            if secret.value.is_empty() {
                return Err(ApiError::bad_request("凭据内容不能为空"));
            }
        }
        Ok(())
    }

    pub fn tags_json(&self) -> ApiResult<String> {
        serde_json::to_string(self.tags.as_deref().unwrap_or(&[]))
            .map_err(|error| ApiError::internal("标签序列化失败").detail(error.to_string()))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnProfile {
    pub id: String,
    pub group_id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub key_path: Option<String>,
    pub key_fingerprint: Option<String>,
    pub agent_identity: Option<String>,
    pub has_password: bool,
    pub has_passphrase: bool,
    pub term: String,
    pub charset: String,
    pub keepalive_secs: i64,
    pub connect_timeout_secs: i64,
    pub jump_profile_id: Option<String>,
    pub tags: Vec<String>,
    pub color: Option<String>,
    pub notes: Option<String>,
    pub sort_order: i64,
    pub last_connected_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

// ---------------- 主机公钥 ----------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyPolicy {
    Tofu,
    Strict,
}

// ---------------- 设置 ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: u8,
    pub line_height: f32,
    pub scrollback: u32,
    pub cursor_blink: bool,
    pub cursor_style: String,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: "\"JetBrains Mono\", \"SF Mono\", Menlo, Consolas, monospace".into(),
            font_size: 13,
            line_height: 1.25,
            scrollback: 10_000,
            cursor_blink: true,
            cursor_style: "bar".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BehaviorSettings {
    pub confirm_on_close_tab: bool,
    pub auto_reconnect: bool,
    #[serde(default = "default_true")]
    pub restore_sessions: bool,
    pub max_retries: u8,
    pub paste_warn_bytes: u32,
    pub editor_max_bytes: u32,
}

fn default_true() -> bool {
    true
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            confirm_on_close_tab: true,
            auto_reconnect: true,
            restore_sessions: true,
            max_retries: 5,
            paste_warn_bytes: 65_536,
            editor_max_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SftpSettings {
    pub show_hidden: bool,
    pub follow_terminal: bool,
    pub preserve_mode: bool,
    pub conflict_policy: String,
    pub download_dir: String,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            show_hidden: false,
            follow_terminal: true,
            preserve_mode: false,
            conflict_policy: "ask".into(),
            download_dir: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorSettings {
    pub interval_ms: u64,
    pub enabled_on_connect: bool,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            interval_ms: 2000,
            enabled_on_connect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecuritySettings {
    pub host_key_policy: String,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            host_key_policy: "tofu".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    pub sidebar_width: u32,
    pub dock_width: u32,
    pub monitor_height: u32,
    pub monitor_collapsed: bool,
    pub sidebar_collapsed: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            sidebar_width: 300,
            dock_width: 400,
            monitor_height: 240,
            monitor_collapsed: false,
            sidebar_collapsed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: String,
    pub terminal: TerminalSettings,
    pub behavior: BehaviorSettings,
    pub sftp: SftpSettings,
    pub monitor: MonitorSettings,
    pub security: SecuritySettings,
    pub ui: UiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            terminal: TerminalSettings::default(),
            behavior: BehaviorSettings::default(),
            sftp: SftpSettings::default(),
            monitor: MonitorSettings::default(),
            security: SecuritySettings::default(),
            ui: UiSettings::default(),
        }
    }
}

/// 部分更新：未提供的顶层 section 保持不变；提供了则整体替换该 section。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub terminal: Option<TerminalSettings>,
    pub behavior: Option<BehaviorSettings>,
    pub sftp: Option<SftpSettings>,
    pub monitor: Option<MonitorSettings>,
    pub security: Option<SecuritySettings>,
    pub ui: Option<UiSettings>,
}

impl SettingsPatch {
    pub fn validate(&self) -> ApiResult<()> {
        if let Some(theme) = &self.theme {
            if theme != "dark" && theme != "light" {
                return Err(ApiError::bad_request("theme 只能是 dark 或 light"));
            }
        }
        if let Some(behavior) = &self.behavior {
            if behavior.max_retries > 10 {
                return Err(ApiError::bad_request("maxRetries 需在 0–10 之间"));
            }
        }
        if let Some(monitor) = &self.monitor {
            if !(1000..=10_000).contains(&monitor.interval_ms) {
                return Err(ApiError::bad_request("监控间隔需在 1000–10000ms 之间"));
            }
        }
        if let Some(security) = &self.security {
            if security.host_key_policy != "tofu" && security.host_key_policy != "strict" {
                return Err(ApiError::bad_request("hostKeyPolicy 只能是 tofu 或 strict"));
            }
        }
        if let Some(terminal) = &self.terminal {
            if !(11..=18).contains(&terminal.font_size) {
                return Err(ApiError::bad_request("终端字号需在 11–18 之间"));
            }
        }
        Ok(())
    }
}

// ---------------- 认证输入 ----------------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSource {
    Inline,
    Stored,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum AuthInput {
    Password {
        source: CredentialSource,
        #[serde(default)]
        password: Option<String>,
    },
    Key {
        source: CredentialSource,
        #[serde(rename = "keyPath")]
        key_path: String,
        #[serde(default)]
        passphrase: Option<String>,
    },
    Agent {
        #[serde(default, rename = "identityPath")]
        identity_path: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
}

/// 解析后的连接凭据（仅存活于内存，不实现 Debug）。
#[derive(Clone)]
pub enum ResolvedAuth {
    Password {
        password: Zeroizing<String>,
    },
    Key {
        key_path: String,
        passphrase: Option<Zeroizing<String>>,
    },
    Agent {
        #[allow(dead_code)] // B2: ssh-agent 身份选择
        identity_path: Option<String>,
    },
}

impl ResolvedAuth {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Password { .. } => "password",
            Self::Key { .. } => "key",
            Self::Agent { .. } => "agent",
        }
    }
}
