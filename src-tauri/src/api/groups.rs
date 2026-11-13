//! 分组命令。

use tauri::State;

use crate::db::models::Group;
use crate::db::repo;
use crate::db::repo::groups::{self, DeleteStrategy, GroupDeleteResult};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[tauri::command]
pub fn group_list(state: State<'_, AppState>) -> ApiResult<Vec<Group>> {
    state.db()?.with(repo::groups::list)
}

#[tauri::command]
pub fn group_create(
    state: State<'_, AppState>,
    parent_id: Option<String>,
    name: String,
) -> ApiResult<Group> {
    state
        .db()?
        .with(|conn| repo::groups::create(conn, parent_id.as_deref(), &name))
}

#[tauri::command]
pub fn group_update(state: State<'_, AppState>, id: String, name: String) -> ApiResult<Group> {
    state
        .db()?
        .with(|conn| repo::groups::rename(conn, &id, &name))
}

#[tauri::command]
pub fn group_delete(
    state: State<'_, AppState>,
    id: String,
    children_strategy: Option<String>,
) -> ApiResult<GroupDeleteResult> {
    let strategy = match children_strategy.as_deref().unwrap_or("move-to-parent") {
        "move-to-parent" => DeleteStrategy::MoveToParent,
        "delete-all" => DeleteStrategy::DeleteAll,
        other => {
            return Err(ApiError::bad_request(format!(
                "未知的 childrenStrategy: {other}"
            )))
        }
    };
    state
        .db()?
        .with_mut(|conn| groups::delete(conn, &id, strategy))
}

#[tauri::command]
pub fn group_move(
    state: State<'_, AppState>,
    id: String,
    parent_id: Option<String>,
    index: usize,
) -> ApiResult<()> {
    state
        .db()?
        .with_mut(|conn| groups::move_group(conn, &id, parent_id.as_deref(), index))
}
