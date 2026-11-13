//! 统一错误模型：所有 Tauri 命令返回 `Result<T, ApiError>`。
//!
//! 与 `docs/02-ipc-contract.md` §2 对齐：
//! - `code` 为稳定的机器可读枚举（前端据此分支处理）；
//! - `message` 可直接展示给用户，禁止包含密码/私钥/口令；
//! - `detail` 放技术细节，仅在“详情”里展示，不进 toast。

use serde::Serialize;

/// 错误码（序列化为 SCREAMING_SNAKE_CASE，与 TS 端 `ErrorCode` 一一对应）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // 请求与通用
    IpcBadRequest,
    Cancelled,
    Internal,

    // SSH
    SshTcp,
    SshHandshake,
    SshTimeout,
    SshChannel,
    SshClosed,
    SshAuthFailed,
    SshAuthUnsupported,
    SshAuthAgentEmpty,
    SshHostkeyMismatch,
    SshHostkeyUnknown,
    SshReconnecting,
    SshReconnectExhausted,
    SessionNotFound,
    SessionExists,

    // SFTP / 传输
    SftpNotReady,
    SftpIo,
    SftpNotFound,
    SftpPermission,
    SftpConflict,
    SftpTooLarge,
    SftpBinary,
    TransferCancelled,
    TransferIo,

    // 转发 / 监控
    ForwardBind,
    ForwardStart,
    ForwardRemoteDenied,
    MonitorUnsupported,
    MonitorTimeout,

    // 配置 / 存储
    ProfileNotFound,
    GroupNotFound,
    GroupCycle,
    DuplicateName,
    SecretNotFound,
    KeychainError,
    KeychainUnavailable,
    ImportParse,
    ImportEmpty,
    ImportVersion,
    DbError,
    DbLocked,
    DbMigration,
    InvalidPath,
    PermissionDenied,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 是否值得自动重试（网络类 true；认证/参数类 false）。
    pub retryable: bool,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            retryable: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IpcBadRequest, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        // 约束冲突单独提示，避免前端只能显示 "database error"。
        let text = error.to_string();
        if text.contains("UNIQUE constraint failed") {
            return Self::new(ErrorCode::DuplicateName, "名称已存在，请换一个").detail(text);
        }
        Self::new(ErrorCode::DbError, "数据库操作失败").detail(text)
    }
}

impl From<keyring::Error> for ApiError {
    fn from(error: keyring::Error) -> Self {
        match error {
            keyring::Error::NoEntry => Self::new(ErrorCode::SecretNotFound, "系统中没有保存该凭据"),
            _ => {
                Self::new(ErrorCode::KeychainError, "系统钥匙串访问失败").detail(error.to_string())
            }
        }
    }
}

impl From<getrandom::Error> for ApiError {
    fn from(error: getrandom::Error) -> Self {
        Self::internal("生成随机密钥失败").detail(error.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self::internal("文件操作失败").detail(error.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal("JSON 序列化失败").detail(error.to_string())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
