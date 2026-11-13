//! Tauri SSH 命令层：参数校验 → 凭据解析 → 建连 → 交给读写循环。
//!
//! B1 已支持：密码（inline / 钥匙串）、私钥（inline / 钥匙串口令）。
//! ssh-agent 与自动重连在后续提交补齐（契约见 docs/02 §3.5 / §5.2）。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use russh::client;
#[cfg(unix)]
use russh::keys::agent::client::AgentClient;
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::db::models::{
    now_ms, AuthInput, AuthType, ConnectTarget, CredentialSource, HostKeyPolicy, ResolvedAuth,
};
use crate::db::repo;
use crate::db::Db;
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::secret::{passphrase_account, password_account, SecretStore};
use crate::state::AppState;

use super::events;
use super::handler::{ClientHandler, FingerprintSlot};
use super::reconnect::ReconnectContext;
use super::session::{self, EstablishedSession, SessionCmd};

const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub session_id: String,
    /// 已保存配置；与 `target` 二选一。
    pub profile_id: Option<String>,
    /// 临时连接（快速连接）。
    pub target: Option<ConnectTarget>,
    /// 省略时按 profile 的认证方式与已存凭据连接。
    pub auth: Option<AuthInput>,
    pub term: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub host_key_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResult {
    pub session_id: String,
    pub auth_method: String,
    pub server_fingerprint: String,
    pub latency_ms: i64,
}

#[derive(Clone)]
pub(crate) struct ConnectionPlan {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) auth: ResolvedAuth,
    pub(crate) term: String,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) timeout_secs: u64,
    pub(crate) keepalive_secs: i64,
    pub(crate) policy: HostKeyPolicy,
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    req: ConnectRequest,
) -> ApiResult<ConnectResult> {
    if req.session_id.trim().is_empty() {
        return Err(ApiError::bad_request("会话 ID 不能为空"));
    }
    if state.sessions().contains(&req.session_id) {
        return Err(ApiError::new(
            ErrorCode::SessionExists,
            "相同会话 ID 已存在，请勿重复连接",
        ));
    }

    let db = state.db()?.clone();
    let secrets = state.secrets()?;
    let plan = resolve_plan(&db, &secrets, &req)?;

    events::status(
        &app,
        &req.session_id,
        "connecting",
        Some(format!("正在连接 {}:{} …", plan.host, plan.port)),
    );

    let started = Instant::now();
    let timeout = Duration::from_secs(plan.timeout_secs);

    let (established, fingerprint) = match tokio::time::timeout(
        timeout,
        establish(
            &app,
            &req.session_id,
            &db,
            &plan,
            state.forwards().targets(),
        ),
    )
    .await
    {
        Err(_) => {
            let error = ApiError::new(
                ErrorCode::SshTimeout,
                format!("连接超时（{} 秒）", plan.timeout_secs),
            )
            .retryable();
            events::status(&app, &req.session_id, "error", Some(error.message.clone()));
            return Err(error);
        }
        Ok(Err(error)) => {
            events::status(&app, &req.session_id, "error", Some(error.message.clone()));
            return Err(error);
        }
        Ok(Ok(value)) => value,
    };

    let (tx, rx) = session::channel();
    state
        .sessions()
        .register(req.session_id.clone(), tx, established.handle.clone());

    events::status(
        &app,
        &req.session_id,
        "connected",
        Some(format!("{}@{}:{}", plan.username, plan.host, plan.port)),
    );

    let auto_reconnect = db
        .with(repo::settings::get)
        .map(|settings| settings.behavior.auto_reconnect)
        .unwrap_or(true);
    let reconnect = if auto_reconnect {
        Some(ReconnectContext {
            manager: state.reconnects().clone(),
            db: db.clone(),
            entry: state.reconnects().register(
                &req.session_id,
                plan.clone(),
                req.profile_id.clone(),
            ),
        })
    } else {
        None
    };

    session::spawn_read_loop(
        app,
        req.session_id.clone(),
        state.sessions().clone(),
        state.forwards().clone(),
        reconnect,
        established,
        rx,
    );

    if let Some(profile_id) = &req.profile_id {
        let _ = db.with(|conn| repo::profiles::touch_last_connected(conn, profile_id, now_ms()));
    }

    Ok(ConnectResult {
        session_id: req.session_id,
        auth_method: plan.auth.method_name().to_string(),
        server_fingerprint: fingerprint,
        latency_ms: started.elapsed().as_millis() as i64,
    })
}

#[tauri::command]
pub fn ssh_write(state: State<'_, AppState>, session_id: String, data: String) -> ApiResult<()> {
    if data.is_empty() {
        return Ok(());
    }
    let bytes = STANDARD
        .decode(data.as_bytes())
        .map_err(|_| ApiError::bad_request("输入数据不是合法的 base64"))?;
    state.sessions().send(&session_id, SessionCmd::Data(bytes))
}

#[tauri::command]
pub fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> ApiResult<()> {
    let cols = cols.clamp(20, MAX_COLS);
    let rows = rows.clamp(5, MAX_ROWS);
    state.reconnects().update_size(&session_id, cols, rows);
    state
        .sessions()
        .send(&session_id, SessionCmd::Resize { cols, rows })
}

#[tauri::command]
pub fn ssh_disconnect(state: State<'_, AppState>, session_id: String) -> ApiResult<()> {
    state.reconnects().cancel(&session_id);
    let _ = state.sessions().send(&session_id, SessionCmd::Close);
    state.sessions().remove(&session_id);
    Ok(())
}

// ---------------- 凭据解析 ----------------

fn resolve_plan(
    db: &Db,
    secrets: &Arc<dyn SecretStore>,
    req: &ConnectRequest,
) -> ApiResult<ConnectionPlan> {
    let profile = match (&req.profile_id, &req.target) {
        (Some(_), Some(_)) => return Err(ApiError::bad_request("profileId 与 target 只能二选一")),
        (None, None) => return Err(ApiError::bad_request("必须提供 profileId 或 target")),
        (Some(id), None) => Some(db.with(|conn| repo::profiles::get(conn, id))?),
        (None, Some(_)) => None,
    };

    let settings = db.with(repo::settings::get)?;

    let policy = match req.host_key_policy.as_deref() {
        Some("strict") => HostKeyPolicy::Strict,
        Some("tofu") => HostKeyPolicy::Tofu,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "未知的 hostKeyPolicy: {other}"
            )))
        }
        None => {
            if settings.security.host_key_policy == "strict" {
                HostKeyPolicy::Strict
            } else {
                HostKeyPolicy::Tofu
            }
        }
    };

    let (host, port, username) = match (profile.as_ref(), req.target.as_ref()) {
        (Some(profile), _) => (profile.host.clone(), profile.port, profile.username.clone()),
        (None, Some(target)) => (
            target.host.trim().to_string(),
            target.port,
            target.username.trim().to_string(),
        ),
        _ => unreachable!("profile/target 已在上方校验"),
    };

    if host.is_empty() {
        return Err(ApiError::bad_request("主机地址不能为空"));
    }
    if port == 0 {
        return Err(ApiError::bad_request("端口号必须大于 0"));
    }
    if username.is_empty() {
        return Err(ApiError::bad_request("用户名不能为空"));
    }

    let auth = match &req.auth {
        Some(input) => resolve_explicit(input, profile.as_ref(), secrets)?,
        None => resolve_from_profile(
            profile
                .as_ref()
                .ok_or_else(|| ApiError::bad_request("临时连接必须显式提供 auth"))?,
            secrets,
        )?,
    };

    let term = req
        .term
        .clone()
        .or_else(|| profile.as_ref().map(|value| value.term.clone()))
        .unwrap_or_else(|| "xterm-256color".to_string());

    let timeout_secs = profile
        .as_ref()
        .map(|value| value.connect_timeout_secs.max(5) as u64)
        .unwrap_or(20);
    let keepalive_secs = profile
        .as_ref()
        .map(|value| value.keepalive_secs)
        .unwrap_or(30);

    Ok(ConnectionPlan {
        host,
        port,
        username,
        auth,
        term,
        cols: req.cols.clamp(20, MAX_COLS),
        rows: req.rows.clamp(5, MAX_ROWS),
        timeout_secs,
        keepalive_secs,
        policy,
    })
}

fn resolve_explicit(
    input: &AuthInput,
    profile: Option<&crate::db::models::ConnProfile>,
    secrets: &Arc<dyn SecretStore>,
) -> ApiResult<ResolvedAuth> {
    match input {
        AuthInput::Password { source, password } => match source {
            CredentialSource::Inline => {
                let password = password
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| ApiError::bad_request("请输入密码"))?;
                Ok(ResolvedAuth::Password {
                    password: Zeroizing::new(password.to_string()),
                })
            }
            CredentialSource::Stored => {
                let profile = profile
                    .ok_or_else(|| ApiError::bad_request("使用已存密码时必须提供 profileId"))?;
                if !profile.has_password {
                    return Err(ApiError::new(
                        ErrorCode::SecretNotFound,
                        "该连接尚未保存密码，请先输入并保存",
                    ));
                }
                let password = secrets
                    .get(&password_account(&profile.id))?
                    .ok_or_else(|| {
                        ApiError::new(
                            ErrorCode::SecretNotFound,
                            "钥匙串中没有该连接的密码，请重新输入",
                        )
                    })?;
                Ok(ResolvedAuth::Password {
                    password: Zeroizing::new(password),
                })
            }
        },
        AuthInput::Key {
            source,
            key_path,
            passphrase,
        } => {
            let path = if key_path.trim().is_empty() {
                profile
                    .and_then(|value| value.key_path.clone())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| ApiError::bad_request("私钥认证必须提供私钥路径"))?
            } else {
                key_path.trim().to_string()
            };

            let passphrase = match source {
                CredentialSource::Inline => passphrase.clone().map(Zeroizing::new),
                CredentialSource::Stored => {
                    let profile = profile
                        .ok_or_else(|| ApiError::bad_request("使用已存口令时必须提供 profileId"))?;
                    if profile.has_passphrase {
                        Some(Zeroizing::new(
                            secrets
                                .get(&passphrase_account(&profile.id))?
                                .ok_or_else(|| {
                                    ApiError::new(
                                        ErrorCode::SecretNotFound,
                                        "钥匙串中没有该私钥的口令，请重新输入",
                                    )
                                })?,
                        ))
                    } else {
                        None
                    }
                }
            };

            Ok(ResolvedAuth::Key {
                key_path: path,
                passphrase,
            })
        }
        AuthInput::Agent { identity_path } => Ok(ResolvedAuth::Agent {
            identity_path: identity_path.clone(),
        }),
    }
}

fn resolve_from_profile(
    profile: &crate::db::models::ConnProfile,
    secrets: &Arc<dyn SecretStore>,
) -> ApiResult<ResolvedAuth> {
    match profile.auth_type {
        AuthType::Password => {
            if !profile.has_password {
                return Err(ApiError::new(
                    ErrorCode::SecretNotFound,
                    "该连接尚未保存密码，请在连接时输入",
                ));
            }
            let password = secrets
                .get(&password_account(&profile.id))?
                .ok_or_else(|| {
                    ApiError::new(
                        ErrorCode::SecretNotFound,
                        "钥匙串中没有该连接的密码，请重新输入",
                    )
                })?;
            Ok(ResolvedAuth::Password {
                password: Zeroizing::new(password),
            })
        }
        AuthType::Key => {
            let key_path = profile
                .key_path
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ApiError::bad_request("该连接未配置私钥路径"))?;
            let passphrase = if profile.has_passphrase {
                Some(Zeroizing::new(
                    secrets
                        .get(&passphrase_account(&profile.id))?
                        .ok_or_else(|| {
                            ApiError::new(
                                ErrorCode::SecretNotFound,
                                "钥匙串中没有该私钥的口令，请重新输入",
                            )
                        })?,
                ))
            } else {
                None
            };
            Ok(ResolvedAuth::Key {
                key_path,
                passphrase,
            })
        }
        AuthType::Agent => Ok(ResolvedAuth::Agent {
            identity_path: profile.agent_identity.clone(),
        }),
    }
}

// ---------------- 建连 ----------------

pub(crate) async fn establish(
    app: &AppHandle,
    session_id: &str,
    db: &Db,
    plan: &ConnectionPlan,
    forwarded: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, (String, u16)>>>,
) -> ApiResult<(EstablishedSession, String)> {
    let config = Arc::new(client::Config {
        // 交互式长连接不能被“空闲超时”回收；用 keepalive 探活。
        inactivity_timeout: None,
        keepalive_interval: if plan.keepalive_secs > 0 {
            Some(Duration::from_secs(plan.keepalive_secs as u64))
        } else {
            None
        },
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let fingerprint: FingerprintSlot = Arc::new(Mutex::new(None));
    let handler = ClientHandler::new(
        app.clone(),
        session_id.to_string(),
        plan.host.clone(),
        plan.port,
        db.clone(),
        plan.policy,
        fingerprint.clone(),
        forwarded,
    );

    let mut handle = client::connect(config, (plan.host.as_str(), plan.port), handler)
        .await
        .map_err(ApiError::from)?;

    match &plan.auth {
        ResolvedAuth::Password { password } => {
            let result = handle
                .authenticate_password(plan.username.clone(), password.as_str())
                .await
                .map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthFailed, "认证请求失败")
                        .detail(error.to_string())
                })?;
            if !result.success() {
                return Err(ApiError::new(
                    ErrorCode::SshAuthFailed,
                    "认证失败：用户名/密码错误，或服务器未开启密码登录",
                ));
            }
        }
        ResolvedAuth::Key {
            key_path,
            passphrase,
        } => {
            let key = load_secret_key(key_path, passphrase.as_ref().map(|value| value.as_str()))
                .map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthFailed, "私钥加载失败")
                        .detail(error.to_string())
                })?;

            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthFailed, "协商 RSA 签名算法失败")
                        .detail(error.to_string())
                })?
                .flatten();

            let result = handle
                .authenticate_publickey(
                    plan.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthFailed, "私钥认证请求失败")
                        .detail(error.to_string())
                })?;
            if !result.success() {
                return Err(ApiError::new(
                    ErrorCode::SshAuthFailed,
                    "私钥认证失败：服务器拒绝了该密钥",
                ));
            }
        }
        ResolvedAuth::Agent { .. } => {
            #[cfg(unix)]
            {
                let mut agent = AgentClient::connect_env().await.map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthAgentEmpty, "无法连接 ssh-agent")
                        .detail(error.to_string())
                })?;
                let identities = agent.request_identities().await.map_err(|error| {
                    ApiError::new(ErrorCode::SshAuthAgentEmpty, "读取 ssh-agent 身份失败")
                        .detail(error.to_string())
                })?;
                if identities.is_empty() {
                    return Err(ApiError::new(
                        ErrorCode::SshAuthAgentEmpty,
                        "ssh-agent 中没有可用身份，请先 ssh-add",
                    ));
                }

                let hash = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|error| {
                        ApiError::new(ErrorCode::SshAuthFailed, "协商 RSA 签名算法失败")
                            .detail(error.to_string())
                    })?
                    .flatten();

                let mut accepted = false;
                for identity in &identities {
                    let key = identity.public_key().into_owned();
                    match handle
                        .authenticate_publickey_with(plan.username.clone(), key, hash, &mut agent)
                        .await
                    {
                        Ok(result) if result.success() => {
                            accepted = true;
                            break;
                        }
                        Ok(_) => continue,
                        Err(error) => {
                            return Err(ApiError::new(
                                ErrorCode::SshAuthFailed,
                                "ssh-agent 签名失败",
                            )
                            .detail(error.to_string()))
                        }
                    }
                }
                if !accepted {
                    return Err(ApiError::new(
                        ErrorCode::SshAuthFailed,
                        "服务器拒绝了 ssh-agent 中的所有身份",
                    ));
                }
            }
            #[cfg(not(unix))]
            {
                return Err(ApiError::new(
                    ErrorCode::SshAuthUnsupported,
                    "当前平台暂不支持 ssh-agent 认证",
                ));
            }
        }
    }

    let channel = handle.channel_open_session().await.map_err(|error| {
        ApiError::new(ErrorCode::SshChannel, "打开会话通道失败").detail(error.to_string())
    })?;

    channel
        .request_pty(
            true,
            &plan.term,
            plan.cols as u32,
            plan.rows as u32,
            0,
            0,
            &[],
        )
        .await
        .map_err(|error| {
            ApiError::new(ErrorCode::SshChannel, "申请 PTY 失败").detail(error.to_string())
        })?;

    channel.request_shell(true).await.map_err(|error| {
        ApiError::new(ErrorCode::SshChannel, "启动远程 shell 失败").detail(error.to_string())
    })?;

    let (read, write) = channel.split();

    let fingerprint = fingerprint
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_default();

    Ok((
        EstablishedSession {
            handle: Arc::new(handle),
            read,
            write,
        },
        fingerprint,
    ))
}
