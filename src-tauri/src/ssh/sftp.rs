//! SFTP 命令：复用同一条 SSH 连接，懒打开 `sftp` 子系统。
//!
//! 范围（B2a）：浏览 / 读取 / 原子写入 / 新建 / 重命名 / 删除 / chmod。
//! 上传下载与进度事件在 B2b 补齐。

use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use russh::client::Handle;
use russh::ChannelMsg;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};
use tauri_plugin_dragout::{DragItem, DragOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::ssh::handler::ClientHandler;
use crate::state::AppState;

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_READ_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 5000;
const MAX_DELETE_DEPTH: usize = 64;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub mode: u32,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<SftpEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpOpenResult {
    pub sftp_session_id: String,
    pub home: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpFileContent {
    pub path: String,
    /// base64；前端用 TextDecoder 还原 UTF-8。
    pub content: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub mode: u32,
    pub is_binary: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileRequest {
    pub session_id: String,
    pub path: String,
    pub content: String,
    pub expected_mtime_ms: Option<i64>,
    pub expected_size: Option<u64>,
    #[serde(default)]
    pub create_backup: bool,
}

pub(crate) fn map_error(context: &str, error: impl std::fmt::Display) -> ApiError {
    let detail = error.to_string();
    let lower = detail.to_lowercase();
    let code = if lower.contains("no such file") || lower.contains("not found") {
        ErrorCode::SftpNotFound
    } else if lower.contains("permission") {
        ErrorCode::SftpPermission
    } else {
        ErrorCode::SftpIo
    };
    ApiError::new(code, context).detail(detail)
}

pub(crate) fn ensure_remote_path(path: &str) -> ApiResult<String> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(ApiError::bad_request("远端路径必须是绝对路径"));
    }
    if path.contains('\0') || path.contains('\n') {
        return Err(ApiError::bad_request("远端路径包含非法字符"));
    }
    let mut normalized = path.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        normalized = "/".to_string();
    }
    Ok(normalized)
}

fn parent_of(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    match path.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(index) => Some(path[..index].to_string()),
        None => None,
    }
}

pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub(crate) fn base_name(path: &str) -> String {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("/")
        .to_string()
}

fn kind_of(attributes: &FileAttributes) -> &'static str {
    if attributes.is_dir() {
        "dir"
    } else if attributes.is_symlink() {
        "symlink"
    } else if attributes.is_regular() {
        "file"
    } else {
        "other"
    }
}

fn mtime_ms(attributes: &FileAttributes) -> i64 {
    attributes
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn mode_of(attributes: &FileAttributes) -> u32 {
    attributes.permissions.unwrap_or(0) & 0o7777
}

fn entry_from(attributes: &FileAttributes, name: &str, path: String) -> SftpEntry {
    SftpEntry {
        name: name.to_string(),
        path,
        kind: kind_of(attributes).to_string(),
        size: attributes.len(),
        mtime_ms: mtime_ms(attributes),
        mode: mode_of(attributes),
        uid: attributes.uid,
        gid: attributes.gid,
    }
}

/// 懒打开 SFTP 子系统：同一会话复用，失败不缓存半成品。
pub(crate) async fn ensure_sftp_from(
    slot: &crate::ssh::session::SftpSlot,
    handle: Arc<russh::client::Handle<super::handler::ClientHandler>>,
) -> ApiResult<Arc<SftpSession>> {
    let mut guard = slot.lock().await;
    if let Some(existing) = guard.as_ref() {
        return Ok(existing.clone());
    }

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| map_error("打开 SFTP 通道失败", error))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| map_error("请求 SFTP 子系统失败", error))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| map_error("初始化 SFTP 会话失败", error))?;

    let sftp = Arc::new(sftp);
    *guard = Some(sftp.clone());
    Ok(sftp)
}

async fn ensure_sftp(state: &AppState, session_id: &str) -> ApiResult<Arc<SftpSession>> {
    let slot = state.sessions().sftp_slot(session_id)?;
    let handle = state.sessions().handle(session_id)?;
    ensure_sftp_from(&slot, handle).await
}

#[tauri::command]
pub async fn sftp_open(
    state: State<'_, AppState>,
    session_id: String,
) -> ApiResult<SftpOpenResult> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let home = sftp
        .canonicalize(".")
        .await
        .map_err(|error| map_error("读取远端 home 失败", error))?;
    Ok(SftpOpenResult {
        sftp_session_id: session_id,
        home,
    })
}

#[tauri::command]
pub async fn sftp_close(state: State<'_, AppState>, session_id: String) -> ApiResult<()> {
    let slot = state.sessions().sftp_slot(&session_id)?;
    let mut guard = slot.lock().await;
    if let Some(sftp) = guard.take() {
        let _ = sftp.close().await;
    }
    Ok(())
}

/// SFTP `read_dir` 在部分服务器/权限边界上会返回 NO_SUCH_FILE，但同一 SSH
/// 会话的 shell `ls/find` 可读。作为列表 fallback，不改变后续文件操作仍走 SFTP。
fn parse_find_entries(root: &str, output: &[u8], show_hidden: bool) -> Vec<SftpEntry> {
    let mut entries = Vec::new();
    let fields: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    for chunk in fields.chunks(5) {
        if chunk.len() < 5 {
            continue;
        }
        let name = String::from_utf8_lossy(chunk[0]).to_string();
        if name.is_empty() || (!show_hidden && name.starts_with('.')) {
            continue;
        }
        let kind = match chunk[1].first().copied() {
            Some(b'd') => "dir",
            Some(b'l') => "symlink",
            _ => "file",
        };
        let size = String::from_utf8_lossy(chunk[2])
            .parse::<u64>()
            .unwrap_or(0);
        let mtime_ms = String::from_utf8_lossy(chunk[3])
            .parse::<f64>()
            .map(|seconds| (seconds * 1000.0) as i64)
            .unwrap_or(0);
        let mode = u32::from_str_radix(String::from_utf8_lossy(chunk[4]).trim(), 8).unwrap_or(0);
        entries.push(SftpEntry {
            name: name.clone(),
            path: join_remote(root, &name),
            kind: kind.to_string(),
            size,
            mtime_ms,
            mode,
            uid: None,
            gid: None,
        });
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
    }
    entries
}

async fn list_via_exec(
    handle: &Handle<ClientHandler>,
    root: &str,
    show_hidden: bool,
) -> ApiResult<Vec<SftpEntry>> {
    let escaped = root.replace('\'', "'\\''");
    let command = format!(
        "LC_ALL=C find -H '{}' -mindepth 1 -maxdepth 1 -printf '%f\\0%y\\0%s\\0%T@\\0%m\\0' 2>/dev/null",
        escaped
    );
    let mut channel = handle.channel_open_session().await.map_err(|error| {
        ApiError::new(ErrorCode::SshChannel, "打开目录列表通道失败").detail(error.to_string())
    })?;
    channel.exec(true, command).await.map_err(|error| {
        ApiError::new(ErrorCode::SftpIo, "执行目录列表 fallback 失败").detail(error.to_string())
    })?;
    let mut output = Vec::new();
    let read = async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(8), read)
        .await
        .map_err(|_| ApiError::new(ErrorCode::SftpIo, "目录列表 fallback 超时"))?;
    let entries = parse_find_entries(root, &output, show_hidden);
    if entries.is_empty() {
        return Err(ApiError::new(
            ErrorCode::SftpIo,
            format!("读取目录失败：{root}"),
        ));
    }
    Ok(entries)
}

fn resolve_link_target(parent: &str, target: &str) -> String {
    if target.starts_with('/') {
        target.to_string()
    } else {
        join_remote(parent, target)
    }
}

fn finish_entries(path: &str, mut entries: Vec<SftpEntry>, max: usize) -> SftpListing {
    let truncated = entries.len() > max;
    entries.truncate(max);
    entries.sort_by(|left, right| {
        let left_dir = left.kind == "dir";
        let right_dir = right.kind == "dir";
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    SftpListing {
        parent: parent_of(path),
        path: path.to_string(),
        entries,
        truncated,
    }
}

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    show_hidden: Option<bool>,
) -> ApiResult<SftpListing> {
    let handle = state.sessions().handle(&session_id)?;
    let slot = state.sessions().sftp_slot(&session_id)?;
    let mut sftp = ensure_sftp(&state, &session_id).await?;
    let requested = ensure_remote_path(&path)?;
    // realpath 只用于把 symlink/相对路径规范化；部分服务器会对可读目录返回
    // NO_SUCH_FILE（例如目录本身是 symlink 或权限边界）。列表失败不应因此中断。
    let path = match sftp.canonicalize(&requested).await {
        Ok(path) => path,
        Err(_) => requested,
    };

    let show_hidden = show_hidden.unwrap_or(false);
    let mut entries = Vec::new();
    let read_dir = match sftp.read_dir(&path).await {
        Ok(entries) => Some(entries),
        Err(first) => {
            // 目录可能在 cd/刷新瞬间被重建；短暂重试。
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            // 部分服务器 READDIR 出错后会卡住 SFTP 子系统；丢弃旧会话重开。
            let mut guard = slot.lock().await;
            if let Some(old) = guard.take() {
                let _ = old.close().await;
            }
            drop(guard);
            match ensure_sftp(&state, &session_id).await {
                Ok(fresh) => {
                    sftp = fresh;
                    match sftp.read_dir(&path).await {
                        Ok(entries) => Some(entries),
                        Err(_) => {
                            // 目录本身是 symlink 时，部分服务器 OPENDIR 会失败；
                            // readlink 拿目标后再 read_dir（展示/操作仍用原路径）。
                            if let Ok(target) = sftp.read_link(&path).await {
                                let resolved = resolve_link_target(&path, &target);
                                if resolved != path {
                                    if let Ok(read_dir) = sftp.read_dir(&resolved).await {
                                        let mut symlink_entries = Vec::new();
                                        for entry in read_dir {
                                            let name = entry.file_name();
                                            if !show_hidden && name.starts_with('.') {
                                                continue;
                                            }
                                            let attributes = entry.metadata();
                                            symlink_entries.push(entry_from(
                                                &attributes,
                                                &name,
                                                join_remote(&path, &name),
                                            ));
                                        }
                                        return Ok(finish_entries(
                                            &path,
                                            symlink_entries,
                                            MAX_LIST_ENTRIES,
                                        ));
                                    }
                                }
                            }
                            entries = list_via_exec(&handle, &path, show_hidden).await.map_err(
                                |error| {
                                    ApiError::new(
                                        ErrorCode::SftpIo,
                                        format!("读取目录失败：{path}"),
                                    )
                                    .detail(format!(
                                        "{}; sftp first error: {}",
                                        error.detail.unwrap_or_default(),
                                        first
                                    ))
                                },
                            )?;
                            None
                        }
                    }
                }
                Err(_) => {
                    entries =
                        list_via_exec(&handle, &path, show_hidden)
                            .await
                            .map_err(|error| {
                                ApiError::new(ErrorCode::SftpIo, format!("读取目录失败：{path}"))
                                    .detail(format!(
                                        "{}; sftp first error: {}",
                                        error.detail.unwrap_or_default(),
                                        first
                                    ))
                            })?;
                    None
                }
            }
        }
    };

    if let Some(read_dir) = read_dir {
        for entry in read_dir {
            let name = entry.file_name();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let attributes = entry.metadata();
            entries.push(entry_from(&attributes, &name, join_remote(&path, &name)));
        }
    }

    let truncated = entries.len() > MAX_LIST_ENTRIES;
    entries.truncate(MAX_LIST_ENTRIES);
    entries.sort_by(|left, right| {
        let left_dir = left.kind == "dir";
        let right_dir = right.kind == "dir";
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    Ok(SftpListing {
        parent: parent_of(&path),
        path,
        entries,
        truncated,
    })
}

/// `cd` 启发式候选校验：SFTP stat 跟随 symlink；非目录/不存在返回 false。
#[tauri::command]
pub async fn sftp_is_dir(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> ApiResult<bool> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;
    match sftp.metadata(&path).await {
        Ok(attributes) => Ok(attributes.is_dir()),
        _ => Ok(false),
    }
}

#[tauri::command]
pub async fn sftp_stat(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> ApiResult<SftpEntry> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;
    let attributes = sftp
        .symlink_metadata(&path)
        .await
        .map_err(|error| map_error("读取文件属性失败", error))?;
    Ok(entry_from(&attributes, &base_name(&path), path))
}

#[tauri::command]
pub async fn sftp_realpath(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> ApiResult<String> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;
    sftp.canonicalize(&path)
        .await
        .map_err(|error| map_error("解析远端路径失败", error))
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> ApiResult<()> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;
    sftp.create_dir(&path)
        .await
        .map_err(|error| map_error("新建目录失败", error))
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: String,
    from: String,
    to: String,
) -> ApiResult<()> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let from = ensure_remote_path(&from)?;
    let to = ensure_remote_path(&to)?;
    sftp.rename(&from, &to)
        .await
        .map_err(|error| map_error("重命名失败", error))
}

#[tauri::command]
pub async fn sftp_delete(
    state: State<'_, AppState>,
    session_id: String,
    paths: Vec<String>,
    recursive: bool,
) -> ApiResult<u64> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let mut deleted = 0_u64;
    for path in paths {
        let path = ensure_remote_path(&path)?;
        deleted += remove_path(&sftp, &path, recursive, 0).await?;
    }
    Ok(deleted)
}

async fn remove_path(
    sftp: &SftpSession,
    path: &str,
    recursive: bool,
    depth: usize,
) -> ApiResult<u64> {
    if depth > MAX_DELETE_DEPTH {
        return Err(ApiError::bad_request("目录层级过深，已中止删除"));
    }
    let attributes = sftp
        .symlink_metadata(path)
        .await
        .map_err(|error| map_error("读取待删除项失败", error))?;

    if attributes.is_dir() {
        if !recursive {
            return Err(ApiError::bad_request("目录需要递归删除"));
        }
        let children: Vec<String> = sftp
            .read_dir(path)
            .await
            .map_err(|error| map_error("读取待删除目录失败", error))?
            .filter(|entry| entry.file_name() != "." && entry.file_name() != "..")
            .map(|entry| entry.path())
            .collect();
        let mut deleted = 0_u64;
        for child in children {
            deleted += Box::pin(remove_path(sftp, &child, recursive, depth + 1)).await?;
        }
        sftp.remove_dir(path)
            .await
            .map_err(|error| map_error("删除目录失败", error))?;
        Ok(deleted + 1)
    } else {
        sftp.remove_file(path)
            .await
            .map_err(|error| map_error("删除文件失败", error))?;
        Ok(1)
    }
}

#[tauri::command]
pub async fn sftp_chmod(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    mode: u32,
) -> ApiResult<()> {
    if mode > 0o7777 {
        return Err(ApiError::bad_request("权限值必须在 0–07777 之间"));
    }
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;
    let attributes = FileAttributes {
        permissions: Some(mode),
        ..Default::default()
    };
    sftp.set_metadata(&path, attributes)
        .await
        .map_err(|error| map_error("修改权限失败", error))
}

#[tauri::command]
pub async fn sftp_read_file(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    max_bytes: Option<u64>,
) -> ApiResult<SftpFileContent> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let path = ensure_remote_path(&path)?;

    let attributes = sftp
        .symlink_metadata(&path)
        .await
        .map_err(|error| map_error("读取文件属性失败", error))?;
    if attributes.is_dir() {
        return Err(ApiError::bad_request("不能以文件方式读取目录"));
    }

    let limit = max_bytes.unwrap_or(DEFAULT_READ_BYTES).min(MAX_FILE_BYTES);
    let size = attributes.len();
    let truncated = size > limit;

    let file = sftp
        .open(&path)
        .await
        .map_err(|error| map_error("打开文件失败", error))?;
    let mut reader = file.take(limit);
    let mut buffer = Vec::with_capacity(size.min(limit) as usize);
    reader
        .read_to_end(&mut buffer)
        .await
        .map_err(|error| map_error("读取文件失败", error))?;

    let is_binary = buffer.iter().take(8192).any(|byte| *byte == 0);
    Ok(SftpFileContent {
        path,
        content: STANDARD.encode(&buffer),
        size,
        mtime_ms: mtime_ms(&attributes),
        mode: mode_of(&attributes),
        is_binary,
        truncated,
    })
}

#[tauri::command]
pub async fn sftp_write_file(
    state: State<'_, AppState>,
    req: WriteFileRequest,
) -> ApiResult<SftpEntry> {
    let sftp = ensure_sftp(&state, &req.session_id).await?;
    let path = ensure_remote_path(&req.path)?;
    let bytes = STANDARD
        .decode(req.content.as_bytes())
        .map_err(|_| ApiError::bad_request("文件内容不是合法的 base64"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ApiError::new(
            ErrorCode::SftpTooLarge,
            "文件超过在线编辑上限（8MB）",
        ));
    }

    let existing = sftp.symlink_metadata(&path).await.ok();
    if let Some(existing) = &existing {
        if let Some(expected_mtime) = req.expected_mtime_ms {
            if mtime_ms(existing) != expected_mtime {
                return Err(ApiError::new(
                    ErrorCode::SftpConflict,
                    "文件已被其他程序修改，请重新加载后再保存",
                ));
            }
        }
        if let Some(expected_size) = req.expected_size {
            if existing.len() != expected_size {
                return Err(ApiError::new(
                    ErrorCode::SftpConflict,
                    "文件大小已变化，请重新加载后再保存",
                ));
            }
        }
    }

    let mode = existing
        .as_ref()
        .map(mode_of)
        .filter(|mode| *mode != 0)
        .unwrap_or(0o644);

    let backup = if req.create_backup && existing.is_some() {
        let backup_path = format!("{path}.nestterm.bak");
        let _ = sftp.remove_file(&backup_path).await;
        sftp.rename(&path, &backup_path)
            .await
            .map_err(|error| map_error("创建备份失败", error))?;
        Some(backup_path)
    } else {
        None
    };

    let temp_path = format!(
        "{path}.nestterm.{}.tmp",
        &uuid::Uuid::now_v7().simple().to_string()[..8]
    );

    let write_result = async {
        let mut file = sftp
            .create(&temp_path)
            .await
            .map_err(|error| map_error("创建临时文件失败", error))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| map_error("写入临时文件失败", error))?;
        file.flush()
            .await
            .map_err(|error| map_error("刷新临时文件失败", error))?;
        let _ = file.shutdown().await;

        let attributes = FileAttributes {
            permissions: Some(mode),
            ..Default::default()
        };
        let _ = sftp.set_metadata(&temp_path, attributes).await;

        sftp.rename(&temp_path, &path)
            .await
            .map_err(|error| map_error("替换目标文件失败", error))?;
        Ok::<(), ApiError>(())
    }
    .await;

    if let Err(error) = write_result {
        let _ = sftp.remove_file(&temp_path).await;
        if let Some(backup) = &backup {
            let _ = sftp.rename(backup, &path).await;
        }
        return Err(error);
    }

    let attributes = sftp
        .symlink_metadata(&path)
        .await
        .map_err(|error| map_error("写入后读取属性失败", error))?;
    Ok(entry_from(&attributes, &base_name(&path), path))
}

async fn copy_remote_to_file(
    sftp: &SftpSession,
    remote: &str,
    destination: &Path,
) -> Result<(), String> {
    let mut source = sftp
        .open(remote)
        .await
        .map_err(|error| format!("打开远端文件失败：{error}"))?;
    let mut target = tokio::fs::File::create(destination)
        .await
        .map_err(|error| format!("创建本地文件失败：{error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取远端文件失败：{error}"))?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("写入本地文件失败：{error}"))?;
    }
    target
        .flush()
        .await
        .map_err(|error| format!("写入本地文件失败：{error}"))?;
    Ok(())
}

async fn download_remote_tree(
    sftp: &SftpSession,
    remote_root: &str,
    local_root: &Path,
) -> Result<(), String> {
    tokio::fs::create_dir_all(local_root)
        .await
        .map_err(|error| format!("创建本地目录失败：{error}"))?;
    let mut stack = vec![(remote_root.to_string(), local_root.to_path_buf())];
    while let Some((remote_dir, local_dir)) = stack.pop() {
        let entries = sftp
            .read_dir(&remote_dir)
            .await
            .map_err(|error| format!("读取远端目录失败：{error}"))?;
        for entry in entries {
            let name = entry.file_name();
            let attributes = entry.metadata();
            if attributes.is_symlink() {
                continue;
            }
            let remote = join_remote(&remote_dir, &name);
            let local = local_dir.join(&name);
            if attributes.is_dir() {
                tokio::fs::create_dir_all(&local)
                    .await
                    .map_err(|error| format!("创建本地目录失败：{error}"))?;
                stack.push((remote, local));
            } else if attributes.is_regular() {
                copy_remote_to_file(sftp, &remote, &local).await?;
            }
        }
    }
    Ok(())
}

/// 原生拖出：按住 `⠿` 拖到 Finder，drop 时才下载。
/// 文件直接写目标路径；目录递归创建并下载。macOS 才支持。
#[tauri::command]
pub async fn sftp_drag_out(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    remote_path: String,
) -> ApiResult<()> {
    let sftp = ensure_sftp(&state, &session_id).await?;
    let remote = ensure_remote_path(&remote_path)?;
    let attributes = sftp
        .symlink_metadata(&remote)
        .await
        .map_err(|error| map_error("读取远端路径失败", error))?;
    if !attributes.is_dir() && !attributes.is_regular() {
        return Err(ApiError::bad_request("只支持文件或目录"));
    }
    let is_dir = attributes.is_dir();
    let name = base_name(&remote);
    let sftp_for_writer = sftp.clone();
    let remote_for_writer = remote.clone();
    let write = move |destination: &Path| -> Result<(), String> {
        let sftp = sftp_for_writer.clone();
        let remote = remote_for_writer.clone();
        let destination = destination.to_path_buf();
        tauri::async_runtime::block_on(async move {
            if is_dir {
                download_remote_tree(&sftp, &remote, &destination).await
            } else {
                copy_remote_to_file(&sftp, &remote, &destination).await
            }
        })
    };
    let item = DragItem::promise(name, write);
    tauri_plugin_dragout::start_drag(&window, vec![item], DragOptions::default()).map_err(
        |error| ApiError::new(ErrorCode::Internal, "启动拖拽失败").detail(error.to_string()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_path_validation_and_normalization() {
        assert_eq!(ensure_remote_path("/var/log/").unwrap(), "/var/log");
        assert_eq!(ensure_remote_path("/").unwrap(), "/");
        assert!(ensure_remote_path("relative/path").is_err());
        assert!(ensure_remote_path("").is_err());
        assert!(ensure_remote_path("/tmp/\0x").is_err());
    }

    #[test]
    fn parent_and_join_paths() {
        assert_eq!(parent_of("/var/log"), Some("/var".into()));
        assert_eq!(parent_of("/var"), Some("/".into()));
        assert_eq!(parent_of("/"), None);
        assert_eq!(join_remote("/", "tmp"), "/tmp");
        assert_eq!(join_remote("/var", "log"), "/var/log");
        assert_eq!(base_name("/var/log/nginx.conf"), "nginx.conf");
    }

    #[test]
    fn file_kind_and_mode_mask() {
        let mut attributes = FileAttributes::default();
        attributes.set_dir(true);
        attributes.permissions = attributes.permissions.map(|mode| mode | 0o755);
        assert_eq!(kind_of(&attributes), "dir");
        assert_eq!(mode_of(&attributes), 0o755);

        let mut regular = FileAttributes::default();
        regular.set_regular(true);
        regular.permissions = regular.permissions.map(|mode| mode | 0o600);
        assert_eq!(kind_of(&regular), "file");
        assert_eq!(mode_of(&regular), 0o600);
    }

    #[test]
    fn mtime_of_missing_returns_zero() {
        let attributes = FileAttributes::default();
        assert_eq!(mtime_ms(&attributes), 0);
    }

    #[test]
    fn parses_find_fallback_entries() {
        let output = b"a b.txt\0f\0123\01700000000.5\0644\0.hidden\0f\01\01700000001\0600\0link\0l\00\01700000002\0777\0";
        let entries = parse_find_entries("/home/demo", output, false);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a b.txt");
        assert_eq!(entries[0].kind, "file");
        assert_eq!(entries[0].size, 123);
        assert_eq!(entries[0].mtime_ms, 1_700_000_000_500);
        assert_eq!(entries[0].mode, 0o644);
        assert_eq!(entries[0].path, "/home/demo/a b.txt");
        assert_eq!(entries[1].kind, "symlink");

        let all = parse_find_entries("/home/demo", output, true);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].name, ".hidden");
        assert_eq!(all[2].kind, "symlink");
    }

    #[test]
    fn resolves_symlink_targets() {
        assert_eq!(resolve_link_target("/a/b", "/x/y"), "/x/y");
        assert_eq!(resolve_link_target("/a/b", "c"), "/a/b/c");
    }
}
