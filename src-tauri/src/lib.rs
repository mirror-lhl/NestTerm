//! NestTerm 应用入口。
//!
//! 分层约定（单人 OPC 迭代时保持清晰边界）：
//! - `api`      Tauri 命令层（配置/分组/设置/known_hosts）；
//! - `db`       SQLCipher 存储 + 仓储；
//! - `secret`   系统钥匙串封装；
//! - `ssh`      SSH 会话与 PTY（russh）；
//! - `state`    全局状态装配（启动失败不 panic，错误回传前端）。

mod api;
mod db;
mod error;
mod portable;
mod secret;
mod ssh;
mod state;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_dragout::init())
        .setup(|app| {
            let state = AppState::initialize(app.handle());
            if let Some(error) = state.startup_error() {
                log::error!("启动初始化失败: {error:?}");
            }
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::app_info,
            ssh::monitor_commands::monitor_sample,
            ssh::forward_commands::forward_start,
            ssh::forward_commands::forward_stop,
            ssh::forward_commands::forward_rules_list,
            ssh::forward_commands::forward_rule_create,
            ssh::forward_commands::forward_rule_delete,
            ssh::forward_commands::forward_running,
            api::session_state::sessions_save,
            api::session_state::sessions_saved,
            portable::nestterm::nestterm_export,
            portable::nestterm::nestterm_import,
            portable::finalshell::finalshell_preview,
            portable::finalshell::finalshell_import,
            api::known_hosts_list,
            api::known_hosts_remove,
            api::group_list,
            api::group_create,
            api::group_update,
            api::group_delete,
            api::group_move,
            api::profile_list,
            api::profile_get,
            api::profile_create,
            api::profile_update,
            api::profile_delete,
            api::profile_duplicate,
            api::profile_move,
            api::profile_set_secret,
            api::profile_clear_secret,
            api::settings_get,
            api::settings_update,
            ssh::sftp::sftp_open,
            ssh::sftp::sftp_close,
            ssh::sftp::sftp_list,
            ssh::sftp::sftp_stat,
            ssh::sftp::sftp_realpath,
            ssh::sftp::sftp_mkdir,
            ssh::sftp::sftp_rename,
            ssh::sftp::sftp_delete,
            ssh::sftp::sftp_drag_out,
            ssh::sftp::sftp_is_dir,
            ssh::sftp::sftp_chmod,
            ssh::sftp::sftp_read_file,
            ssh::sftp::sftp_write_file,
            ssh::transfer::sftp_upload,
            ssh::transfer::sftp_download,
            ssh::transfer::sftp_cancel_transfer,
            ssh::commands::ssh_connect,
            ssh::commands::ssh_write,
            ssh::commands::ssh_resize,
            ssh::commands::ssh_disconnect,
        ])
        .run(tauri::generate_context!())
        .expect("NestTerm 启动失败");
}

/// 初始化日志。默认 info；排查握手问题用：
/// `RUST_LOG=russh=debug npm run app:dev`
fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
}
