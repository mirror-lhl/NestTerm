//! russh 客户端回调实现。
//!
//! 安全基线（对齐 OpenSSH 的 accept-new 策略，记录改为应用自有 SQLite 表）：
//! - 与 known_hosts 表一致 → 放行；
//! - 首次见到该主机 → 记录指纹（TOFU）后放行，并通过事件提示用户；
//! - 指纹已存在但不匹配 → 直接拒绝握手（疑似中间人/服务器重装）。
//! - `hostKeyPolicy=strict` 时，未知主机也拒绝。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use russh::client::{self, ChannelOpenHandle};
use russh::keys::{HashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure};
use tauri::AppHandle;

use crate::db::models::HostKeyPolicy;
use crate::db::repo;
use crate::db::Db;
use crate::error::{ApiError, ErrorCode};

use super::events;
use super::forward::bridge;

pub type FingerprintSlot = Arc<Mutex<Option<String>>>;

/// `client::Handler::Error` 需要同时满足 `From<russh::Error>`，故用 thiserror 统一封装。
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error(transparent)]
    Ssh(#[from] russh::Error),
    #[error("{message}")]
    App {
        code: ErrorCode,
        message: String,
        detail: Option<String>,
    },
}

impl From<HandlerError> for ApiError {
    fn from(error: HandlerError) -> Self {
        match error {
            HandlerError::Ssh(error) => ApiError::new(ErrorCode::SshHandshake, "SSH 握手失败")
                .detail(error.to_string())
                .retryable(),
            HandlerError::App {
                code,
                message,
                detail,
            } => {
                let mut api_error = ApiError::new(code, message);
                if let Some(detail) = detail {
                    api_error = api_error.detail(detail);
                }
                api_error
            }
        }
    }
}

pub struct ClientHandler {
    app: AppHandle,
    session_id: String,
    host: String,
    port: u16,
    db: Db,
    policy: HostKeyPolicy,
    fingerprint: FingerprintSlot,
    /// 远端转发端口 -> 本地目标；仅由 ForwardManager 写入。
    forwarded: Arc<Mutex<HashMap<u32, (String, u16)>>>,
}

impl ClientHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: AppHandle,
        session_id: String,
        host: String,
        port: u16,
        db: Db,
        policy: HostKeyPolicy,
        fingerprint: FingerprintSlot,
        forwarded: Arc<Mutex<HashMap<u32, (String, u16)>>>,
    ) -> Self {
        Self {
            app,
            session_id,
            host,
            port,
            db,
            policy,
            fingerprint,
            forwarded,
        }
    }
}

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // 证书登录尚未启用：若服务端下发的是 OpenSSH 证书，取其中的公钥做指纹校验。
        // 注意 ssh-key 0.7 中 Certificate::public_key() 返回 KeyData，需转换为 PublicKey。
        let key: PublicKey = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(certificate) => {
                PublicKey::from(certificate.public_key().clone())
            }
        };

        let key_type = key.algorithm().as_str().to_string();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let public_key = key.to_openssh().unwrap_or_default();

        if let Ok(mut slot) = self.fingerprint.lock() {
            *slot = Some(fingerprint.clone());
        }

        let strict = self.policy == HostKeyPolicy::Strict;
        let outcome = self
            .db
            .with(|conn| {
                repo::known_hosts::verify(
                    conn,
                    &self.host,
                    self.port,
                    &key_type,
                    &fingerprint,
                    &public_key,
                    strict,
                )
            })
            .map_err(|error| HandlerError::App {
                code: error.code,
                message: error.message,
                detail: error.detail,
            })?;

        let trusted = outcome == repo::known_hosts::VerifyOutcome::Trusted;
        events::host_key(
            &self.app,
            &self.session_id,
            &fingerprint,
            &key_type,
            trusted,
            !trusted,
        );
        Ok(true)
    }

    /// 服务器为 `-R` 打开的入站连接：查端口映射，连本地目标并桥接。
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<russh::client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let target = self
            .forwarded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&connected_port)
            .cloned();
        let Some((host, port)) = target else {
            let _ = reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        match tokio::net::TcpStream::connect((host.as_str(), port)).await {
            Ok(socket) => {
                let _ = reply.accept().await;
                tokio::spawn(async move {
                    let _ = bridge(socket, channel).await;
                });
            }
            Err(_) => {
                let _ = reply.reject(ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }

    /// 把服务端登录 banner 直接写进终端，方便看到“未授权访问”之类的提示。
    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let text = format!("\r\n\x1b[33m{}\x1b[0m\r\n", banner.trim_end());
        events::data(&self.app, &self.session_id, text.as_bytes());
        Ok(())
    }
}
