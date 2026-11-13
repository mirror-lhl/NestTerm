//! 连接配置命令：CRUD + 凭据写入（只写不读）。

use serde::Serialize;
use tauri::State;

use crate::db::models::{ConnProfile, ProfileInput, SecretInput};
use crate::db::repo;
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::secret::{passphrase_account, password_account};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProfilesResult {
    pub deleted: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFlagResult {
    pub has_secret: bool,
}

fn account_for(kind: &str, profile_id: &str) -> ApiResult<String> {
    match kind {
        "password" => Ok(password_account(profile_id)),
        "passphrase" => Ok(passphrase_account(profile_id)),
        _ => Err(ApiError::bad_request("未知的凭据类型")),
    }
}

fn store_secret(state: &AppState, profile_id: &str, secret: &SecretInput) -> ApiResult<()> {
    let account = account_for(&secret.kind, profile_id)?;
    let store = state.secrets()?;
    store.set(&account, &secret.value)?;

    if let Err(error) = state
        .db()?
        .with(|conn| repo::profiles::set_secret_flag(conn, profile_id, &secret.kind, true))
    {
        // 补偿：数据库标记失败时不要把凭据留在钥匙串里。
        let _ = store.delete(&account);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn profile_list(state: State<'_, AppState>) -> ApiResult<Vec<ConnProfile>> {
    state
        .db()?
        .with(|conn| repo::profiles::list(conn, repo::profiles::GroupFilter::All))
}

#[tauri::command]
pub fn profile_get(state: State<'_, AppState>, id: String) -> ApiResult<ConnProfile> {
    state.db()?.with(|conn| repo::profiles::get(conn, &id))
}

#[tauri::command]
pub fn profile_create(state: State<'_, AppState>, input: ProfileInput) -> ApiResult<ConnProfile> {
    let db = state.db()?;
    let profile = db.with(|conn| repo::profiles::create(conn, &input))?;

    if let Some(secret) = &input.secret {
        if let Err(error) = store_secret(&state, &profile.id, secret) {
            let _ = db.with_mut(|conn| {
                repo::profiles::delete_many(conn, std::slice::from_ref(&profile.id))
            });
            return Err(error);
        }
    }

    db.with(|conn| repo::profiles::get(conn, &profile.id))
}

/// B1 为全量更新：前端持有完整记录后提交。`secret` 为 `None` 时保留已存凭据。
#[tauri::command]
pub fn profile_update(
    state: State<'_, AppState>,
    id: String,
    input: ProfileInput,
) -> ApiResult<ConnProfile> {
    let db = state.db()?;
    let profile = db.with(|conn| repo::profiles::update(conn, &id, &input))?;

    if let Some(secret) = &input.secret {
        store_secret(&state, &profile.id, secret)?;
    }

    db.with(|conn| repo::profiles::get(conn, &id))
}

#[tauri::command]
pub fn profile_delete(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> ApiResult<DeleteProfilesResult> {
    let db = state.db()?;
    let store = state.secrets()?;

    let mut touched: Vec<(String, bool, bool)> = Vec::new();
    for id in &ids {
        let profile = match db.with(|conn| repo::profiles::get(conn, id)) {
            Ok(profile) => profile,
            Err(error) if error.code == ErrorCode::ProfileNotFound => continue,
            Err(error) => return Err(error),
        };

        // 先删钥匙串：失败则中止，避免“看不见但仍存在”的凭据。
        if profile.has_password {
            store.delete(&password_account(id))?;
        }
        if profile.has_passphrase {
            store.delete(&passphrase_account(id))?;
        }
        touched.push((id.clone(), profile.has_password, profile.has_passphrase));
    }

    match db.with_mut(|conn| repo::profiles::delete_many(conn, &ids)) {
        Ok(deleted) => Ok(DeleteProfilesResult { deleted }),
        Err(error) => {
            // 补偿：DB 删除失败时把凭据标记清零，避免 UI 显示“已保存”。
            for (id, has_password, has_passphrase) in &touched {
                if *has_password {
                    let _ = db
                        .with(|conn| repo::profiles::set_secret_flag(conn, id, "password", false));
                }
                if *has_passphrase {
                    let _ = db.with(|conn| {
                        repo::profiles::set_secret_flag(conn, id, "passphrase", false)
                    });
                }
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub fn profile_duplicate(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
) -> ApiResult<ConnProfile> {
    state
        .db()?
        .with(|conn| repo::profiles::duplicate(conn, &id, name.as_deref()))
}

#[tauri::command]
pub fn profile_move(
    state: State<'_, AppState>,
    id: String,
    group_id: Option<String>,
    index: usize,
) -> ApiResult<()> {
    state
        .db()?
        .with_mut(|conn| repo::profiles::move_profile(conn, &id, group_id.as_deref(), index))
}

#[tauri::command]
pub fn profile_set_secret(
    state: State<'_, AppState>,
    profile_id: String,
    kind: String,
    value: String,
) -> ApiResult<SecretFlagResult> {
    let secret = SecretInput {
        kind: kind.clone(),
        value,
    };
    // 先确认配置存在，避免给不存在的 profile 写钥匙串。
    state
        .db()?
        .with(|conn| repo::profiles::get(conn, &profile_id))?;
    store_secret(&state, &profile_id, &secret)?;
    Ok(SecretFlagResult { has_secret: true })
}

#[tauri::command]
pub fn profile_clear_secret(
    state: State<'_, AppState>,
    profile_id: String,
    kind: String,
) -> ApiResult<()> {
    let account = account_for(&kind, &profile_id)?;
    state.secrets()?.delete(&account)?;
    state
        .db()?
        .with(|conn| repo::profiles::set_secret_flag(conn, &profile_id, &kind, false))
}
