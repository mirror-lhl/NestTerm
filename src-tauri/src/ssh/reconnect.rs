//! 后端编排的 same-session 自动重连。
//!
//! 连接成功后把 `ConnectionPlan` 留在内存；读写循环异常/远端 shell 退出时，
//! 由后端按原 sessionId 重建 SSH 连接，并恢复之前正在运行的转发规则。
//! 用户主动断开（关闭标签/调用 ssh_disconnect）会 cancel，不再重试。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use super::commands::{establish, ConnectionPlan};
use super::events;
use super::forward::ForwardManager;
use super::session::{self, SshState};
use crate::db::repo::forward_rules::ForwardRule;
use crate::db::Db;

const MAX_ATTEMPTS: u32 = 3;

pub struct Entry {
    pub plan: Arc<ConnectionPlan>,
    #[allow(dead_code)]
    pub profile_id: Option<String>,
    pub canceled: AtomicBool,
    /// 最近一次 PTY 尺寸；重连时优先使用，而不是首次连接尺寸。
    pub size: Mutex<(u16, u16)>,
}

#[derive(Clone, Default)]
pub struct ReconnectManager {
    inner: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
}

impl ReconnectManager {
    pub fn register(
        &self,
        session_id: &str,
        plan: ConnectionPlan,
        profile_id: Option<String>,
    ) -> Arc<Entry> {
        let size = (plan.cols, plan.rows);
        let entry = Arc::new(Entry {
            plan: Arc::new(plan),
            profile_id,
            canceled: AtomicBool::new(false),
            size: Mutex::new(size),
        });
        self.lock().insert(session_id.to_string(), entry.clone());
        entry
    }

    pub fn update_size(&self, session_id: &str, cols: u16, rows: u16) {
        if let Some(entry) = self.lock().get(session_id) {
            if let Ok(mut size) = entry.size.lock() {
                *size = (cols, rows);
            }
        }
    }

    pub fn cancel(&self, session_id: &str) {
        if let Some(entry) = self.lock().remove(session_id) {
            entry.canceled.store(true, Ordering::SeqCst);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Entry>>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone)]
pub struct ReconnectContext {
    pub manager: ReconnectManager,
    pub db: Db,
    pub entry: Arc<Entry>,
}

/// 重连循环：成功后注册会话、重启 pump、恢复转发规则。
pub async fn run(
    app: AppHandle,
    session_id: String,
    state: SshState,
    forwards: ForwardManager,
    context: ReconnectContext,
    rules: Vec<ForwardRule>,
) {
    for attempt in 1..=MAX_ATTEMPTS {
        if context.entry.canceled.load(Ordering::SeqCst) {
            return;
        }
        events::status(
            &app,
            &session_id,
            "reconnecting",
            Some(format!("连接已断开，第 {attempt}/{MAX_ATTEMPTS} 次重连…")),
        );
        tokio::time::sleep(std::time::Duration::from_millis(700 * attempt as u64)).await;
        if context.entry.canceled.load(Ordering::SeqCst) {
            return;
        }

        let (cols, rows) = context
            .entry
            .size
            .lock()
            .map(|size| *size)
            .unwrap_or((context.entry.plan.cols, context.entry.plan.rows));
        let mut plan = (*context.entry.plan).clone();
        plan.cols = cols;
        plan.rows = rows;
        let established =
            match establish(&app, &session_id, &context.db, &plan, forwards.targets()).await {
                Ok(value) => value.0,
                Err(error) => {
                    log::warn!("重连第 {attempt} 次失败: {}", error.message);
                    if attempt == MAX_ATTEMPTS {
                        events::status(&app, &session_id, "error", Some(error.message));
                        context.manager.cancel(&session_id);
                    }
                    continue;
                }
            };

        if context.entry.canceled.load(Ordering::SeqCst) {
            let _ = established
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
            return;
        }

        let (tx, rx) = session::channel();
        state.register(session_id.clone(), tx, established.handle.clone());
        events::status(
            &app,
            &session_id,
            "connected",
            Some(format!(
                "{}@{}:{}",
                context.entry.plan.username, context.entry.plan.host, context.entry.plan.port
            )),
        );

        session::spawn_read_loop(
            app.clone(),
            session_id.clone(),
            state.clone(),
            forwards.clone(),
            Some(context.clone()),
            established,
            rx,
        );

        for rule in rules {
            let handle = match state.handle(&session_id) {
                Ok(handle) => handle,
                Err(_) => break,
            };
            if let Err(error) = forwards.start(rule, handle, session_id.clone()).await {
                log::warn!("重连后恢复转发失败: {}", error.message);
            }
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{HostKeyPolicy, ResolvedAuth};

    fn plan() -> ConnectionPlan {
        ConnectionPlan {
            host: "192.0.2.10".into(),
            port: 22,
            username: "demo".into(),
            auth: ResolvedAuth::Agent {
                identity_path: None,
            },
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24,
            timeout_secs: 20,
            keepalive_secs: 30,
            policy: HostKeyPolicy::Tofu,
        }
    }

    #[test]
    fn cancel_marks_context_and_removes_entry() {
        let manager = ReconnectManager::default();
        let entry = manager.register("session-1", plan(), None);
        assert!(!entry.canceled.load(Ordering::SeqCst));
        manager.cancel("session-1");
        assert!(entry.canceled.load(Ordering::SeqCst));
        assert!(!manager.lock().contains_key("session-1"));
    }
}
