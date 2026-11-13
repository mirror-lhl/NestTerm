//! 系统钥匙串封装。
//!
//! `/docs/03-database-schema.md` §4 约定：
//! - 服务名固定 `com.nestterm.desktop`；
//! - DB 密钥账户 `db-key-v1`；
//! - 凭据账户 `profile/<id>/password` 与 `profile/<id>/passphrase`；
//! - 凭据只写不读回前端；读取仅在建立连接时发生。
//!
//! 测试用 `MemoryStore`，避免单测依赖真实钥匙串与用户授权弹窗。

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::error::{ApiError, ApiResult, ErrorCode};

pub const SERVICE: &str = "com.nestterm.desktop";
pub const DB_KEY_ACCOUNT: &str = "db-key-v1";

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> ApiResult<Option<String>>;
    fn set(&self, account: &str, value: &str) -> ApiResult<()>;
    fn delete(&self, account: &str) -> ApiResult<()>;
}

pub fn password_account(profile_id: &str) -> String {
    format!("profile/{profile_id}/password")
}

pub fn passphrase_account(profile_id: &str) -> String {
    format!("profile/{profile_id}/passphrase")
}

/// macOS Keychain / Windows Credential Manager / Linux Secret Service。
pub struct KeyringStore;

impl KeyringStore {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> ApiResult<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, account)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(ApiError::from(error)),
        }
    }

    fn set(&self, account: &str, value: &str) -> ApiResult<()> {
        let entry = keyring::Entry::new(SERVICE, account)?;
        entry.set_password(value).map_err(ApiError::from)
    }

    fn delete(&self, account: &str) -> ApiResult<()> {
        let entry = keyring::Entry::new(SERVICE, account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(ApiError::from(error)),
        }
    }
}

/// 仅测试使用：进程内存储，行为与真实钥匙串一致。
#[allow(dead_code)] // 调试模式/单测使用
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> ApiResult<Option<String>> {
        let guard = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(guard.get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> ApiResult<()> {
        let mut guard = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> ApiResult<()> {
        let mut guard = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(account);
        Ok(())
    }
}

/// 读取/生成 SQLCipher 主密钥（32 字节，hex 存钥匙串）。
///
/// debug 构建支持 `NESTTERM_DB_KEY_HEX` 覆盖，便于无钥匙串授权的自动化环境做冒烟测试；
/// release 构建必须走系统钥匙串。
pub fn load_or_create_db_key(store: &dyn SecretStore) -> ApiResult<Zeroizing<Vec<u8>>> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("NESTTERM_DB_KEY_HEX") {
        let bytes = hex::decode(value.trim())
            .map_err(|_| ApiError::bad_request("NESTTERM_DB_KEY_HEX 不是合法 hex"))?;
        if bytes.len() != 32 {
            return Err(ApiError::bad_request("NESTTERM_DB_KEY_HEX 必须是 32 字节"));
        }
        return Ok(Zeroizing::new(bytes));
    }

    if let Some(stored) = store.get(DB_KEY_ACCOUNT)? {
        let stored = Zeroizing::new(stored);
        let bytes = hex::decode(stored.trim())
            .map_err(|_| ApiError::new(ErrorCode::KeychainError, "数据库密钥格式损坏"))?;
        if bytes.len() != 32 {
            return Err(ApiError::new(
                ErrorCode::KeychainError,
                "数据库密钥长度不正确（应为 32 字节）",
            ));
        }
        return Ok(Zeroizing::new(bytes));
    }

    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key)?;
    store.set(DB_KEY_ACCOUNT, &hex::encode(&key))?;
    Ok(Zeroizing::new(key))
}
