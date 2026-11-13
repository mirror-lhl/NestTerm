//! 本地端口转发（-L）与 SOCKS5（-D）。
//!
//! 所有数据复用同一条 SSH 连接：每条本地 TCP 连接对应一个
//! `direct-tcpip` channel。远端 `-R` 尚未实现（需要 server 端转发 channel）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{Handle, Msg};
use russh::Channel;
use std::collections::HashMap as StdHashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::handler::ClientHandler;
use crate::db::repo::forward_rules::ForwardRule;
use crate::error::{ApiError, ApiResult, ErrorCode};

#[derive(Clone, Default)]
pub struct ForwardManager {
    inner: Arc<Mutex<HashMap<String, Running>>>,
    /// 远端转发端口 -> 本地目标；由 ClientHandler 处理 forwarded-tcpip 时读取。
    targets: Arc<Mutex<StdHashMap<u32, (String, u16)>>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
    session_id: String,
    remote: bool,
    rule: ForwardRule,
}

impl ForwardManager {
    pub fn is_running(&self, rule_id: &str) -> bool {
        self.lock().contains_key(rule_id)
    }

    pub fn running_ids(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    pub fn targets(&self) -> Arc<Mutex<StdHashMap<u32, (String, u16)>>> {
        self.targets.clone()
    }

    pub async fn start(
        &self,
        rule: ForwardRule,
        handle: Arc<Handle<ClientHandler>>,
        session_id: String,
    ) -> ApiResult<()> {
        if self.is_running(&rule.id) {
            return Err(ApiError::new(ErrorCode::ForwardStart, "该规则已在运行"));
        }
        if rule.kind == "remote" {
            return self.start_remote(rule, handle, session_id).await;
        }

        let listener = TcpListener::bind((rule.bind_host.as_str(), rule.bind_port))
            .await
            .map_err(|error| {
                ApiError::new(
                    ErrorCode::ForwardBind,
                    format!("无法监听 {}:{}", rule.bind_host, rule.bind_port),
                )
                .detail(error.to_string())
            })?;

        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run_listener(listener, rule.clone(), handle, stop.clone()));

        let mut guard = self.lock();
        if guard.contains_key(&rule.id) {
            stop.store(true, Ordering::SeqCst);
            task.abort();
            return Err(ApiError::new(ErrorCode::ForwardStart, "该规则已在运行"));
        }
        guard.insert(
            rule.id.clone(),
            Running {
                stop,
                task,
                session_id,
                remote: false,
                rule,
            },
        );
        Ok(())
    }

    async fn start_remote(
        &self,
        rule: ForwardRule,
        handle: Arc<Handle<ClientHandler>>,
        session_id: String,
    ) -> ApiResult<()> {
        let target_host = rule
            .target_host
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ApiError::new(ErrorCode::ForwardStart, "远端转发缺少目标主机"))?;
        let target_port = rule
            .target_port
            .filter(|port| *port > 0)
            .ok_or_else(|| ApiError::new(ErrorCode::ForwardStart, "远端转发缺少目标端口"))?;
        if self.lock().contains_key(&rule.id)
            || self.targets_lock().contains_key(&(rule.bind_port as u32))
        {
            return Err(ApiError::new(ErrorCode::ForwardStart, "远端端口已在转发"));
        }

        let actual_port = handle
            .tcpip_forward(rule.bind_host.clone(), rule.bind_port as u32)
            .await
            .map_err(|error| {
                ApiError::new(
                    ErrorCode::ForwardRemoteDenied,
                    format!("服务器拒绝远端转发 {}:{}", rule.bind_host, rule.bind_port),
                )
                .detail(error.to_string())
            })?;
        let actual_port = if actual_port == 0 {
            rule.bind_port as u32
        } else {
            actual_port
        };
        self.targets_lock()
            .insert(actual_port, (target_host, target_port));

        let stop = Arc::new(AtomicBool::new(false));
        let targets = self.targets.clone();
        let bind_host = rule.bind_host.clone();
        let handle_for_task = handle.clone();
        let stop_task = stop.clone();
        let task = tokio::spawn(async move {
            while !stop_task.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            let _ = handle_for_task
                .cancel_tcpip_forward(bind_host, actual_port)
                .await;
            targets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&actual_port);
        });

        self.lock().insert(
            rule.id.clone(),
            Running {
                stop,
                task,
                session_id,
                remote: true,
                rule,
            },
        );
        Ok(())
    }

    pub fn stop(&self, rule_id: &str) -> bool {
        let running = self.lock().remove(rule_id);
        match running {
            Some(running) => {
                running.stop.store(true, Ordering::SeqCst);
                if !running.remote {
                    running.task.abort();
                }
                true
            }
            None => false,
        }
    }

    /// 停止并返回该会话正在运行的规则（重连后按原样重启）。
    pub fn take_session_rules(&self, session_id: &str) -> Vec<ForwardRule> {
        let mut guard = self.lock();
        let ids: Vec<String> = guard
            .iter()
            .filter(|(_, running)| running.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut rules = Vec::new();
        for id in ids {
            if let Some(running) = guard.remove(&id) {
                running.stop.store(true, Ordering::SeqCst);
                if !running.remote {
                    running.task.abort();
                } else {
                    self.targets_lock().remove(&(running.rule.bind_port as u32));
                }
                rules.push(running.rule);
            }
        }
        rules
    }

    fn targets_lock(&self) -> std::sync::MutexGuard<'_, StdHashMap<u32, (String, u16)>> {
        self.targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Running>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

async fn run_listener(
    listener: TcpListener,
    rule: ForwardRule,
    handle: Arc<Handle<ClientHandler>>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            accepted = listener.accept() => {
                match accepted {
                    Ok((socket, _peer)) => {
                        let handle = handle.clone();
                        let rule = rule.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = handle_connection(socket, handle, &rule).await {
                                log::debug!("转发连接结束: {error:?}");
                            }
                        });
                    }
                    Err(error) => {
                        log::warn!("转发监听失败: {error}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    handle: Arc<Handle<ClientHandler>>,
    rule: &ForwardRule,
) -> ApiResult<()> {
    let (target_host, target_port) = if rule.kind == "dynamic" {
        socks5_handshake(&mut socket).await?
    } else {
        (
            rule.target_host.clone().unwrap_or_default(),
            rule.target_port.unwrap_or(0),
        )
    };
    if target_host.is_empty() || target_port == 0 {
        return Err(ApiError::new(ErrorCode::ForwardStart, "转发目标无效"));
    }

    let channel = handle
        .channel_open_direct_tcpip(target_host, target_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|error| {
            ApiError::new(ErrorCode::SshChannel, "打开 direct-tcpip 失败").detail(error.to_string())
        })?;

    bridge(socket, channel).await
}

pub(crate) async fn bridge(socket: TcpStream, channel: Channel<Msg>) -> ApiResult<()> {
    let (mut channel_read, channel_write) = channel.split();
    let mut channel_reader = channel_read.make_reader();
    let mut channel_writer = channel_write.make_writer();
    let (mut socket_read, mut socket_write) = socket.into_split();

    tokio::select! {
        _ = tokio::io::copy(&mut socket_read, &mut channel_writer) => {}
        _ = tokio::io::copy(&mut channel_reader, &mut socket_write) => {}
    }
    let _ = channel_write.eof().await;
    let _ = channel_write.close().await;
    Ok(())
}

async fn socks5_handshake(socket: &mut TcpStream) -> ApiResult<(String, u16)> {
    let mut head = [0_u8; 2];
    socket
        .read_exact(&mut head)
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 握手读取失败"))?;
    if head[0] != 5 {
        return Err(ApiError::new(ErrorCode::ForwardStart, "仅支持 SOCKS5"));
    }
    let mut methods = vec![0_u8; head[1] as usize];
    socket
        .read_exact(&mut methods)
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 方法列表读取失败"))?;
    if !methods.contains(&0) {
        let _ = socket.write_all(&[5, 0xff]).await;
        return Err(ApiError::new(
            ErrorCode::ForwardStart,
            "SOCKS5 客户端不支持 no-auth",
        ));
    }
    socket
        .write_all(&[5, 0])
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 应答失败"))?;

    let mut request = [0_u8; 4];
    socket
        .read_exact(&mut request)
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 请求读取失败"))?;
    if request[0] != 5 || request[1] != 1 {
        let _ = socket.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        return Err(ApiError::new(
            ErrorCode::ForwardStart,
            "只支持 SOCKS5 CONNECT",
        ));
    }

    let host = match request[3] {
        1 => {
            let mut address = [0_u8; 4];
            socket
                .read_exact(&mut address)
                .await
                .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 IPv4 读取失败"))?;
            std::net::Ipv4Addr::from(address).to_string()
        }
        3 => {
            let mut length = [0_u8; 1];
            socket
                .read_exact(&mut length)
                .await
                .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 域名长度读取失败"))?;
            let mut domain = vec![0_u8; length[0] as usize];
            socket
                .read_exact(&mut domain)
                .await
                .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 域名读取失败"))?;
            String::from_utf8_lossy(&domain).to_string()
        }
        4 => {
            let mut address = [0_u8; 16];
            socket
                .read_exact(&mut address)
                .await
                .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 IPv6 读取失败"))?;
            std::net::Ipv6Addr::from(address).to_string()
        }
        _ => {
            let _ = socket.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            return Err(ApiError::new(
                ErrorCode::ForwardStart,
                "不支持 SOCKS5 地址类型",
            ));
        }
    };

    let mut port_bytes = [0_u8; 2];
    socket
        .read_exact(&mut port_bytes)
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 端口读取失败"))?;
    let port = u16::from_be_bytes(port_bytes);
    socket
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| ApiError::new(ErrorCode::ForwardStart, "SOCKS5 成功应答失败"))?;
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_state_is_per_rule() {
        let manager = ForwardManager::default();
        assert!(!manager.is_running("rule-1"));
        assert!(manager.running_ids().is_empty());
        assert!(!manager.stop("rule-1"));
    }
}
