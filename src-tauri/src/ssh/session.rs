//! SSH 会话运行时。
//!
//! 职责边界（刻意保持“笨”）：
//! - 维护 `session_id -> 写通道` 的会话表；
//! - 单个会话的读写循环：远端字节流 → 事件；前端输入 → 远端；
//! - 终端输出做 8ms 合并，避免 vim/tmux 高频重绘时把 IPC 打爆。
//!
//! 不做的事：鉴权、重连、SFTP、监控采集（分别由后续模块负责）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use russh_sftp::client::SftpSession;
use tauri::AppHandle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use super::events;
use super::forward::ForwardManager;
use super::handler::ClientHandler;
use super::reconnect::ReconnectContext;
use crate::error::{ApiError, ApiResult, ErrorCode};

/// 输出合并窗口：8ms 内到达的多个 SSH 数据包合并成一次 IPC。
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
/// 单批上限：大文件 `cat` 时及时落盘到前端，避免内存堆积。
const FLUSH_THRESHOLD: usize = 256 * 1024;

/// 前端 → 后端 的会话指令。
pub enum SessionCmd {
    /// 键盘输入 / 粘贴内容（已由前端按 UTF-8 编码为字节）。
    Data(Vec<u8>),
    /// 终端尺寸变化，必须同步给远端 PTY，否则 vim/tmux 会错行。
    Resize { cols: u16, rows: u16 },
    /// 主动断开。
    Close,
}

/// 已完成握手 + 认证 + PTY 的会话资源。
pub struct EstablishedSession {
    /// 必须随读写循环一起存活：Handle 被 drop 会导致底层连接结束。
    /// 用 Arc 是为了让 SFTP / 监控等命令能借用同一连接开新 channel。
    pub handle: Arc<russh::client::Handle<ClientHandler>>,
    pub read: ChannelReadHalf,
    pub write: ChannelWriteHalf<Msg>,
}

/// SFTP 子系统槽位：懒加载，首次 sftp_open 时创建；同一连接复用。
pub type SftpSlot = Arc<tokio::sync::Mutex<Option<Arc<SftpSession>>>>;

pub struct SessionEntry {
    cmd_tx: UnboundedSender<SessionCmd>,
    handle: Arc<russh::client::Handle<ClientHandler>>,
    sftp: SftpSlot,
}

type SessionMap = HashMap<String, SessionEntry>;

/// 挂在 Tauri managed state 上的会话表（克隆廉价，内部 Arc 共享）。
#[derive(Clone, Default)]
pub struct SshState {
    inner: Arc<Mutex<SessionMap>>,
}

impl SshState {
    fn lock(&self) -> MutexGuard<'_, SessionMap> {
        // 单条会话 panic 不应该让整个会话表不可用：中毒后直接取回内部数据。
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn contains(&self, session_id: &str) -> bool {
        self.lock().contains_key(session_id)
    }

    pub fn register(
        &self,
        session_id: String,
        tx: UnboundedSender<SessionCmd>,
        handle: Arc<russh::client::Handle<ClientHandler>>,
    ) {
        self.lock().insert(
            session_id,
            SessionEntry {
                cmd_tx: tx,
                handle,
                sftp: Arc::new(tokio::sync::Mutex::new(None)),
            },
        );
    }

    /// 取同一 SSH 连接的 Handle（Arc 克隆），用于开 SFTP/监控 channel。
    pub fn handle(&self, session_id: &str) -> ApiResult<Arc<russh::client::Handle<ClientHandler>>> {
        self.lock()
            .get(session_id)
            .map(|entry| entry.handle.clone())
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::SessionNotFound,
                    format!("会话 {session_id} 不存在或已断开"),
                )
            })
    }

    pub fn sftp_slot(&self, session_id: &str) -> ApiResult<SftpSlot> {
        self.lock()
            .get(session_id)
            .map(|entry| entry.sftp.clone())
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::SessionNotFound,
                    format!("会话 {session_id} 不存在或已断开"),
                )
            })
    }

    pub fn remove(&self, session_id: &str) {
        self.lock().remove(session_id);
    }

    pub fn send(&self, session_id: &str, cmd: SessionCmd) -> ApiResult<()> {
        let sessions = self.lock();
        let entry = sessions.get(session_id).ok_or_else(|| {
            ApiError::new(
                ErrorCode::SessionNotFound,
                format!("会话 {session_id} 不存在或已断开"),
            )
        })?;
        entry.cmd_tx.send(cmd).map_err(|_| {
            ApiError::new(
                ErrorCode::SshClosed,
                format!("会话 {session_id} 的读写循环已退出"),
            )
        })
    }
}

/// 启动读写循环（连接建立成功后由命令层调用）。
pub fn spawn_read_loop(
    app: AppHandle,
    session_id: String,
    state: SshState,
    forwards: ForwardManager,
    reconnect: Option<ReconnectContext>,
    mut session: EstablishedSession,
    rx: UnboundedReceiver<SessionCmd>,
) {
    tauri::async_runtime::spawn(async move {
        let result = pump(&app, &session_id, &mut session, rx).await;
        let rules = forwards.take_session_rules(&session_id);
        state.remove(&session_id);

        match result {
            Ok(exit_code) => {
                let message = match exit_code {
                    Some(code) => format!("远端 shell 已退出（退出码 {code}）"),
                    None => "连接已关闭".to_string(),
                };
                events::status(&app, &session_id, "disconnected", Some(message));
            }
            Err(message) => events::status(&app, &session_id, "error", Some(message)),
        }

        let reconnect_active = reconnect
            .as_ref()
            .map(|context| {
                !context
                    .entry
                    .canceled
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
            .unwrap_or(false);
        if reconnect_active {
            if let Some(context) = reconnect {
                tauri::async_runtime::spawn(super::reconnect::run(
                    app.clone(),
                    session_id.clone(),
                    state.clone(),
                    forwards.clone(),
                    context,
                    rules,
                ));
            }
        }
    });
}

/// 会话主循环：同时等待「远端消息」和「前端指令」，任一方向结束即收敛。
async fn pump(
    app: &AppHandle,
    session_id: &str,
    session: &mut EstablishedSession,
    mut rx: UnboundedReceiver<SessionCmd>,
) -> Result<Option<u32>, String> {
    let mut pending: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut flush_at: Option<Instant> = None;
    let mut exit_code: Option<u32> = None;

    loop {
        // 复制一份截止时间给定时分支，避免 async 块与 flush_at 的可变借用冲突。
        let scheduled = flush_at;
        let timer = async move {
            match scheduled {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            // biased + 前端指令优先：保证键盘输入永远先于批量输出被处理。
            biased;

            cmd = rx.recv() => {
                match cmd {
                    Some(SessionCmd::Data(bytes)) => {
                        // 先做输入启发式；路径只是候选，前端会 SFTP realpath 校验。
                        if !bytes.is_empty() {
                            session
                                .write
                                .data_bytes(bytes)
                                .await
                                .map_err(|error| format!("写入远端失败：{error}"))?;
                        }
                    }
                    Some(SessionCmd::Resize { cols, rows }) => {
                        session
                            .write
                            .window_change(cols as u32, rows as u32, 0, 0)
                            .await
                            .map_err(|error| format!("同步窗口大小失败：{error}"))?;
                    }
                    Some(SessionCmd::Close) | None => {
                        flush(app, session_id, &mut pending);
                        let _ = session.write.eof().await;
                        let _ = session
                            .handle
                            .disconnect(russh::Disconnect::ByApplication, "", "en")
                            .await;
                        return Ok(exit_code);
                    }
                }
            }

            message = session.read.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        pending.extend_from_slice(&data);
                        mark_pending(&mut pending, &mut flush_at, app, session_id);
                    }
                    // stderr 同样并入终端，行为与 ssh 一致。
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        pending.extend_from_slice(&data);
                        mark_pending(&mut pending, &mut flush_at, app, session_id);
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        flush(app, session_id, &mut pending);
                        let _ = session.write.close().await;
                        return Ok(exit_code);
                    }
                    Some(_) => {}
                }
            }

            _ = timer => {
                flush(app, session_id, &mut pending);
                flush_at = None;
            }
        }
    }
}

/// 记录待发送数据，并按需触发立即 flush。
fn mark_pending(
    pending: &mut Vec<u8>,
    flush_at: &mut Option<Instant>,
    app: &AppHandle,
    session_id: &str,
) {
    // 只在首个包到达时排定窗口，后续包不延长窗口（避免持续输出导致永不 flush）。
    if flush_at.is_none() {
        *flush_at = Some(Instant::now() + FLUSH_INTERVAL);
    }
    if pending.len() >= FLUSH_THRESHOLD {
        flush(app, session_id, pending);
        *flush_at = None;
    }
}

fn flush(app: &AppHandle, session_id: &str, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    events::data(app, session_id, pending);
    pending.clear();
}

/// 创建写入通道，返回 `(发送端, 接收端)`。
pub fn channel() -> (UnboundedSender<SessionCmd>, UnboundedReceiver<SessionCmd>) {
    mpsc::unbounded_channel()
}
