//! 应用全局状态：数据库、钥匙串、SSH 会话表。
//!
//! 启动时初始化失败不会 panic（避免用户只看到闪退）：
//! 错误记录在 `startup_error`，前端通过 `app_info.startupError` 展示可操作的提示。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use crate::db::Db;
use crate::error::{ApiError, ApiResult, ErrorCode};
#[cfg(debug_assertions)]
use crate::secret::MemoryStore;
use crate::secret::{load_or_create_db_key, KeyringStore, SecretStore};
use crate::ssh::forward::ForwardManager;
use crate::ssh::reconnect::ReconnectManager;
use crate::ssh::SshState;

#[derive(Clone, Default)]
pub struct Transfers {
    inner: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl Transfers {
    pub fn register(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.to_string(), flag.clone());
        flag
    }

    pub fn cancel(&self, id: &str) -> bool {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(flag) = guard.get(id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }
}

pub struct AppState {
    db: Option<Db>,
    secrets: Option<Arc<dyn SecretStore>>,
    startup_error: Option<ApiError>,
    sessions: SshState,
    transfers: Transfers,
    forwards: ForwardManager,
    reconnects: ReconnectManager,
    data_dir: Option<PathBuf>,
}

impl AppState {
    pub fn initialize(app: &AppHandle) -> Self {
        let sessions = SshState::default();
        let store = secret_store();

        let mut state = Self {
            db: None,
            secrets: Some(store.clone()),
            startup_error: None,
            sessions,
            transfers: Transfers::default(),
            forwards: ForwardManager::default(),
            reconnects: ReconnectManager::default(),
            data_dir: None,
        };

        match initialize_storage(app, store) {
            Ok((db, dir)) => {
                state.db = Some(db);
                state.data_dir = Some(dir);
            }
            Err(error) => {
                log::error!("初始化存储失败: {error:?}");
                state.startup_error = Some(error);
            }
        }

        state
    }

    pub fn db(&self) -> ApiResult<&Db> {
        self.db.as_ref().ok_or_else(|| {
            self.startup_error
                .clone()
                .unwrap_or_else(|| ApiError::new(ErrorCode::DbError, "数据库尚未初始化"))
        })
    }

    pub fn secrets(&self) -> ApiResult<Arc<dyn SecretStore>> {
        self.secrets.clone().ok_or_else(|| {
            ApiError::new(
                ErrorCode::KeychainUnavailable,
                "系统钥匙串不可用，无法读写凭据",
            )
        })
    }

    pub fn sessions(&self) -> &SshState {
        &self.sessions
    }

    pub fn transfers(&self) -> &Transfers {
        &self.transfers
    }

    pub fn forwards(&self) -> &ForwardManager {
        &self.forwards
    }

    pub fn reconnects(&self) -> &ReconnectManager {
        &self.reconnects
    }

    pub fn startup_error(&self) -> Option<&ApiError> {
        self.startup_error.as_ref()
    }
}

/// 调试辅助：自动化/沙箱环境可用内存凭据库，规避 Keychain 授权弹窗。
/// release 构建永不启用（`cfg(debug_assertions)`）。
fn secret_store() -> Arc<dyn SecretStore> {
    #[cfg(debug_assertions)]
    if std::env::var("NESTTERM_MEMORY_SECRETS").ok().as_deref() == Some("1") {
        log::warn!("NESTTERM_MEMORY_SECRETS=1：使用内存凭据库（仅调试，重启即丢）");
        return Arc::new(MemoryStore::new());
    }
    Arc::new(KeyringStore::new())
}

fn initialize_storage(app: &AppHandle, store: Arc<dyn SecretStore>) -> ApiResult<(Db, PathBuf)> {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("NESTTERM_DATA_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        let key = load_or_create_db_key(store.as_ref())?;
        let db = Db::open_file(&dir.join("nestterm.db"), &key)?;
        log::info!("数据库已打开（NESTTERM_DATA_DIR）: {:?}", db.path());
        return Ok((db, dir));
    }

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| ApiError::internal("无法定位应用数据目录").detail(error.to_string()))?;
    std::fs::create_dir_all(&dir)?;

    let key = load_or_create_db_key(store.as_ref())?;
    let db = Db::open_file(&dir.join("nestterm.db"), &key)?;
    log::info!("数据库已打开: {:?} (schema v{})", db.path(), db.version());

    // 一次性/幂等迁移：仅针对已有连接配置，从 ~/.ssh/known_hosts 补齐信任。
    match crate::portable::known_hosts_legacy::migrate(&db) {
        Ok(result) if result.imported > 0 => log::info!(
            "known_hosts 迁移：导入 {}，跳过 {}，冲突 {}",
            result.imported,
            result.skipped,
            result.conflicts
        ),
        Ok(_) => {}
        Err(error) => log::warn!("known_hosts 迁移失败（不影响启动）：{error:?}"),
    }

    Ok((db, dir))
}
