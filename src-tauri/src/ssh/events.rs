//! 后端 → 前端的事件出口。
//!
//! 事件名与前端 `src/lib/ssh.ts` 中的订阅一一对应，修改时两侧必须同步：
//! - `ssh:status`  会话状态（connecting / connected / reconnecting / disconnected / error）
//! - `ssh:data`    终端输出（base64，避免 JSON 破坏二进制/多字节 UTF-8）
//! - `ssh:hostkey` 主机公钥指纹（TOFU 结果）

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db::models::now_ms;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusPayload {
    session_id: String,
    state: &'static str,
    message: Option<String>,
    ts: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataPayload {
    session_id: String,
    data: String,
    ts: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostKeyPayload {
    session_id: String,
    fingerprint: String,
    key_type: String,
    trusted: bool,
    learned: bool,
    ts: i64,
}

/// 发送会话状态。`state` 只接受字符串字面量，避免拼写漂移。
pub fn status(app: &AppHandle, session_id: &str, state: &'static str, message: Option<String>) {
    let payload = StatusPayload {
        session_id: session_id.to_owned(),
        state,
        message,
        ts: now_ms(),
    };
    let _ = app.emit("ssh:status", payload);
}

/// 发送终端输出（空切片直接丢弃，避免无意义 IPC）。
pub fn data(app: &AppHandle, session_id: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let payload = DataPayload {
        session_id: session_id.to_owned(),
        data: STANDARD.encode(bytes),
        ts: now_ms(),
    };
    let _ = app.emit("ssh:data", payload);
}

/// 发送主机指纹事件。`trusted=false` 表示首次见到该主机（已 TOFU 记录）。
pub fn host_key(
    app: &AppHandle,
    session_id: &str,
    fingerprint: &str,
    key_type: &str,
    trusted: bool,
    learned: bool,
) {
    let payload = HostKeyPayload {
        session_id: session_id.to_owned(),
        fingerprint: fingerprint.to_owned(),
        key_type: key_type.to_owned(),
        trusted,
        learned,
        ts: now_ms(),
    };
    let _ = app.emit("ssh:hostkey", payload);
}

/// 终端 cwd 变化（OSC 7 解析）。前端据此驱动 SFTP 目录跟随。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgressPayload {
    transfer_id: String,
    session_id: String,
    direction: &'static str,
    local_path: String,
    remote_path: String,
    bytes: u64,
    total_bytes: u64,
    speed_bps: u64,
    files_done: u32,
    files_total: u32,
    ts: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferDonePayload {
    transfer_id: String,
    session_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<crate::error::ApiError>,
    ts: i64,
}

#[allow(clippy::too_many_arguments)]
pub fn transfer_progress(
    app: &AppHandle,
    transfer_id: &str,
    session_id: &str,
    direction: &'static str,
    local_path: &str,
    remote_path: &str,
    bytes: u64,
    total_bytes: u64,
    speed_bps: u64,
    files_done: u32,
    files_total: u32,
) {
    let payload = TransferProgressPayload {
        transfer_id: transfer_id.to_owned(),
        session_id: session_id.to_owned(),
        direction,
        local_path: local_path.to_owned(),
        remote_path: remote_path.to_owned(),
        bytes,
        total_bytes,
        speed_bps,
        files_done,
        files_total,
        ts: now_ms(),
    };
    let _ = app.emit("transfer:progress", payload);
}

pub fn transfer_done(
    app: &AppHandle,
    transfer_id: &str,
    session_id: &str,
    status: &'static str,
    error: Option<crate::error::ApiError>,
) {
    let payload = TransferDonePayload {
        transfer_id: transfer_id.to_owned(),
        session_id: session_id.to_owned(),
        status,
        error,
        ts: now_ms(),
    };
    let _ = app.emit("transfer:done", payload);
}
