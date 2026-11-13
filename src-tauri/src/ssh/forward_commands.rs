//! 端口转发 IPC。

use tauri::State;

use crate::db::repo;
use crate::db::repo::forward_rules::{ForwardRule, ForwardRuleInput};
use crate::error::ApiResult;
use crate::state::AppState;

#[tauri::command]
pub fn forward_rules_list(
    state: State<'_, AppState>,
    profile_id: String,
) -> ApiResult<Vec<ForwardRule>> {
    state
        .db()?
        .with(|conn| repo::forward_rules::list(conn, &profile_id))
}

#[tauri::command]
pub fn forward_rule_create(
    state: State<'_, AppState>,
    input: ForwardRuleInput,
) -> ApiResult<ForwardRule> {
    state
        .db()?
        .with(|conn| repo::forward_rules::create(conn, &input))
}

#[tauri::command]
pub fn forward_rule_delete(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state.forwards().stop(&id);
    state
        .db()?
        .with(|conn| repo::forward_rules::delete(conn, &id))
}

#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    session_id: String,
    rule_id: String,
) -> ApiResult<()> {
    let rule = state
        .db()?
        .with(|conn| repo::forward_rules::get(conn, &rule_id))?;
    let handle = state.sessions().handle(&session_id)?;
    state.forwards().start(rule, handle, session_id).await
}

#[tauri::command]
pub fn forward_stop(state: State<'_, AppState>, rule_id: String) -> ApiResult<()> {
    state.forwards().stop(&rule_id);
    Ok(())
}

#[tauri::command]
pub fn forward_running(state: State<'_, AppState>) -> ApiResult<Vec<String>> {
    Ok(state.forwards().running_ids())
}
