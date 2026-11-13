//! 应用信息与主机公钥管理命令。

use tauri::State;

use crate::db::models::KnownHost;
use crate::db::repo;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub api_version: u32,
    pub db_version: i32,
    pub arch: &'static str,
    pub os: &'static str,
    pub tauri_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_error: Option<ApiError>,
    /// 仅 debug 构建：联调自动化（自动连接指定 profile + 发送初始命令）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev: Option<DevInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevInfo {
    pub auto_connect: Option<String>,
    pub initial_command: Option<String>,
    pub auto_connect_password: Option<String>,
    pub sftp_selftest: bool,
    pub transfer_selftest: bool,
    pub transfer_local_path: Option<String>,
    pub transfer_remote_dir: Option<String>,
    pub transfer_download_dir: Option<String>,
    pub transfer_cancel_local: Option<String>,
    pub import_test: bool,
    pub export_test: bool,
    pub export_path: Option<String>,
    pub json_import_path: Option<String>,
    pub forward_test: bool,
    pub monitor_panel: bool,
    pub settings_panel: bool,
}

pub const API_VERSION: u32 = 1;

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        api_version: API_VERSION,
        db_version: state.db().map(|db| db.version()).unwrap_or(0),
        arch: std::env::consts::ARCH,
        os: std::env::consts::OS,
        tauri_version: tauri::VERSION,
        startup_error: state.startup_error().cloned(),
        dev: dev_info(),
    }
}

fn dev_info() -> Option<DevInfo> {
    #[cfg(debug_assertions)]
    {
        let auto_connect = std::env::var("NESTTERM_DEV_AUTOCONNECT").ok();
        let initial_command = std::env::var("NESTTERM_DEV_INITIAL_COMMAND").ok();
        let sftp_selftest =
            std::env::var("NESTTERM_DEV_SFTP_SELFTEST").ok().as_deref() == Some("1");
        let transfer_selftest = std::env::var("NESTTERM_DEV_TRANSFER_SELFTEST")
            .ok()
            .as_deref()
            == Some("1");
        let import_test = std::env::var("NESTTERM_DEV_IMPORT_TEST").ok().as_deref() == Some("1");
        let export_test = std::env::var("NESTTERM_DEV_EXPORT_TEST").ok().as_deref() == Some("1");
        let json_import_path = std::env::var("NESTTERM_DEV_JSON_IMPORT_PATH").ok();
        let forward_test = std::env::var("NESTTERM_DEV_FORWARD_TEST").ok().as_deref() == Some("1");
        let monitor_panel =
            std::env::var("NESTTERM_DEV_MONITOR_PANEL").ok().as_deref() == Some("1");
        let settings_panel =
            std::env::var("NESTTERM_DEV_SETTINGS_PANEL").ok().as_deref() == Some("1");
        if auto_connect.is_none()
            && initial_command.is_none()
            && !sftp_selftest
            && !transfer_selftest
            && !import_test
            && !export_test
            && json_import_path.is_none()
            && !forward_test
            && !monitor_panel
            && !settings_panel
        {
            None
        } else {
            Some(DevInfo {
                auto_connect,
                initial_command,
                auto_connect_password: std::env::var("NESTTERM_DEV_AUTOCONNECT_PASSWORD").ok(),
                sftp_selftest,
                transfer_selftest,
                transfer_local_path: std::env::var("NESTTERM_DEV_TRANSFER_LOCAL").ok(),
                transfer_remote_dir: std::env::var("NESTTERM_DEV_TRANSFER_REMOTE_DIR").ok(),
                transfer_download_dir: std::env::var("NESTTERM_DEV_TRANSFER_DOWNLOAD_DIR").ok(),
                transfer_cancel_local: std::env::var("NESTTERM_DEV_TRANSFER_CANCEL_LOCAL").ok(),
                import_test,
                export_test,
                export_path: std::env::var("NESTTERM_DEV_EXPORT_PATH").ok(),
                json_import_path,
                forward_test,
                monitor_panel,
                settings_panel,
            })
        }
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

#[tauri::command]
pub fn known_hosts_list(state: State<'_, AppState>) -> ApiResult<Vec<KnownHost>> {
    state.db()?.with(repo::known_hosts::list)
}

#[tauri::command]
pub fn known_hosts_remove(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state
        .db()?
        .with(|conn| repo::known_hosts::remove(conn, &id))
}
