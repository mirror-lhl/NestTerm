use tauri::State;

use super::monitor::{self, MonitorStats};
use crate::error::ApiResult;
use crate::state::AppState;

#[tauri::command]
pub async fn monitor_sample(
    state: State<'_, AppState>,
    session_id: String,
) -> ApiResult<MonitorStats> {
    let handle = state.sessions().handle(&session_id)?;
    monitor::sample(&handle).await
}
