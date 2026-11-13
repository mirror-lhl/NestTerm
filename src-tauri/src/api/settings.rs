//! 设置命令。

use tauri::State;

use crate::db::models::{Settings, SettingsPatch};
use crate::db::repo;
use crate::error::ApiResult;
use crate::state::AppState;

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> ApiResult<Settings> {
    state.db()?.with(repo::settings::get)
}

#[tauri::command]
pub fn settings_update(state: State<'_, AppState>, patch: SettingsPatch) -> ApiResult<Settings> {
    patch.validate()?;
    state
        .db()?
        .with_mut(|conn| repo::settings::update(conn, &patch))
}
