//! 应用退出前的打开会话列表：存 settings KV，重启后恢复。

use tauri::State;

use crate::db::repo;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const KEY: &str = "open_profiles";
const MAX_SESSIONS: usize = 20;

#[tauri::command]
pub fn sessions_saved(state: State<'_, AppState>) -> ApiResult<Vec<String>> {
    let raw = state
        .db()?
        .with(|conn| repo::settings::get_raw(conn, KEY))?;
    let ids: Vec<String> = match raw {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(ids)
}

#[tauri::command]
pub fn sessions_save(state: State<'_, AppState>, profile_ids: Vec<String>) -> ApiResult<()> {
    if profile_ids.len() > MAX_SESSIONS {
        return Err(ApiError::bad_request("恢复会话数量超过上限"));
    }
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<String> = profile_ids
        .into_iter()
        .filter(|id| id.len() <= 64 && !id.is_empty())
        .filter(|id| seen.insert(id.clone()))
        .collect();
    let value = serde_json::to_string(&ids)?;
    state
        .db()?
        .with_mut(|conn| repo::settings::set_raw(conn, KEY, &value))
}
