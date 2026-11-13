//! SFTP 文件传输（单文件）：上传 / 下载 / 进度 / 取消 / 冲突策略。
//!
//! B2b-1 范围：**文件**传输（目录递归在 B2b-2）；冲突策略支持
//! `ask`（由前端确认后重试）/ `overwrite` / `skip` / `rename`。
//! 所有传输复用同一 SSH 连接的 SFTP 子系统，不新开 TCP。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::events;
use super::handler::ClientHandler;
use super::session::SftpSlot;
use super::sftp::{
    base_name, ensure_remote_path, ensure_sftp_from, join_remote, map_error, mode_of,
};
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::state::AppState;

const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadRequest {
    pub session_id: String,
    pub local_paths: Vec<String>,
    pub remote_dir: String,
    pub conflict_policy: Option<String>,
    #[serde(default)]
    pub preserve_mode: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub session_id: String,
    pub remote_paths: Vec<String>,
    pub local_dir: Option<String>,
    pub conflict_policy: Option<String>,
    #[serde(default)]
    pub preserve_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStartResult {
    pub transfer_ids: Vec<String>,
    pub skipped: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Ask,
    Overwrite,
    Skip,
    Rename,
}

fn parse_policy(value: Option<&str>) -> ApiResult<Policy> {
    match value.unwrap_or("ask") {
        "ask" => Ok(Policy::Ask),
        "overwrite" => Ok(Policy::Overwrite),
        "skip" => Ok(Policy::Skip),
        "rename" => Ok(Policy::Rename),
        other => Err(ApiError::bad_request(format!("未知的冲突策略: {other}"))),
    }
}

fn new_transfer_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn with_index(name: &str, index: u32) -> String {
    match name.rfind('.') {
        Some(position) if position > 0 => {
            format!("{} ({index}){}", &name[..position], &name[position..])
        }
        _ => format!("{name} ({index})"),
    }
}

async fn unique_remote_path(sftp: &SftpSession, remote: &str) -> ApiResult<String> {
    let parent = match remote.rfind('/') {
        Some(0) => "/".to_string(),
        Some(position) => remote[..position].to_string(),
        None => return Err(ApiError::bad_request("远端路径必须是绝对路径")),
    };
    let name = base_name(remote);
    for index in 1..=999 {
        let candidate = join_remote(&parent, &with_index(&name, index));
        if sftp.symlink_metadata(&candidate).await.is_err() {
            return Ok(candidate);
        }
    }
    Err(ApiError::new(
        ErrorCode::SftpConflict,
        "同名文件过多，无法自动重命名",
    ))
}

fn unique_local_path(local: &Path) -> ApiResult<PathBuf> {
    let parent = local
        .parent()
        .ok_or_else(|| ApiError::bad_request("本地路径无效"))?;
    let name = local
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::bad_request("本地文件名无效"))?;
    for index in 1..=999 {
        let candidate = parent.join(with_index(name, index));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(ApiError::new(
        ErrorCode::SftpConflict,
        "同名文件过多，无法自动重命名",
    ))
}

fn default_download_dir() -> ApiResult<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| ApiError::new(ErrorCode::InvalidPath, "无法定位用户主目录"))?;
    let dir = PathBuf::from(home).join("Downloads").join("NestTerm");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn conflict_error(paths: &[String]) -> ApiError {
    let preview = paths
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    ApiError::new(
        ErrorCode::SftpConflict,
        format!("{} 个目标已存在，请选择覆盖、跳过或重命名", paths.len()),
    )
    .detail(preview)
}

fn reports_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

fn cancelled_error() -> ApiError {
    ApiError::new(ErrorCode::TransferCancelled, "传输已取消")
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &AppHandle,
    transfer_id: &str,
    session_id: &str,
    direction: &'static str,
    local: &Path,
    remote: &str,
    bytes: u64,
    total: u64,
    started: Instant,
    done: bool,
) {
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let speed = (bytes as f64 / elapsed) as u64;
    events::transfer_progress(
        app,
        transfer_id,
        session_id,
        direction,
        &local.to_string_lossy(),
        remote,
        bytes,
        total,
        speed,
        0,
        1,
    );
    if done {
        events::transfer_progress(
            app,
            transfer_id,
            session_id,
            direction,
            &local.to_string_lossy(),
            remote,
            total.max(bytes),
            total.max(bytes),
            speed,
            1,
            1,
        );
    }
}

const MAX_FILES_PER_REQUEST: usize = 2000;
const MAX_TREE_DEPTH: usize = 64;

/// 本地目录树（不含根）：(绝对路径, 相对路径, 大小, 是否目录)。
fn walk_local_tree(root: &Path) -> ApiResult<Vec<(PathBuf, String, u64, bool)>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0_usize)];
    while let Some((dir, relative, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(ApiError::bad_request("目录层级过深"));
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| ApiError::bad_request("本地文件名包含非 UTF-8 字符"))?
                .to_string();
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let path = entry.path();
            if file_type.is_dir() {
                out.push((path.clone(), child_relative.clone(), 0, true));
                stack.push((path, child_relative, depth + 1));
            } else if file_type.is_file() {
                let size = entry.metadata()?.len();
                out.push((path, child_relative, size, false));
            }
            if out.iter().filter(|entry| !entry.3).count() > MAX_FILES_PER_REQUEST {
                return Err(ApiError::bad_request("单次目录传输文件数超过 2000，请分批"));
            }
        }
    }
    Ok(out)
}

/// 远端目录树（不含根）：(绝对路径, 相对路径, 大小, 是否目录)。
async fn walk_remote_tree(
    sftp: &SftpSession,
    root: &str,
) -> ApiResult<Vec<(String, String, u64, bool)>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_string(), String::new(), 0_usize)];
    while let Some((dir, relative, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(ApiError::bad_request("远端目录层级过深"));
        }
        let entries = sftp
            .read_dir(&dir)
            .await
            .map_err(|error| map_error("读取远端目录失败", error))?;
        for entry in entries {
            let name = entry.file_name();
            let attributes = entry.metadata();
            if attributes.is_symlink() {
                continue;
            }
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let child_path = join_remote(&dir, &name);
            if attributes.is_dir() {
                out.push((child_path.clone(), child_relative.clone(), 0, true));
                stack.push((child_path, child_relative, depth + 1));
            } else if attributes.is_regular() {
                out.push((child_path, child_relative, attributes.len(), false));
            }
            if out.iter().filter(|entry| !entry.3).count() > MAX_FILES_PER_REQUEST {
                return Err(ApiError::bad_request("单次目录传输文件数超过 2000，请分批"));
            }
        }
    }
    Ok(out)
}

fn join_local(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_path_buf(), |path, segment| path.join(segment))
}

// ---------------- 上传 ----------------

#[tauri::command]
pub async fn sftp_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    req: UploadRequest,
) -> ApiResult<TransferStartResult> {
    let policy = parse_policy(req.conflict_policy.as_deref())?;
    let remote_dir = ensure_remote_path(&req.remote_dir)?;
    let slot = state.sessions().sftp_slot(&req.session_id)?;
    let handle = state.sessions().handle(&req.session_id)?;
    let sftp = ensure_sftp_from(&slot, handle.clone()).await?;

    let attributes = sftp
        .metadata(&remote_dir)
        .await
        .map_err(|error| map_error("目标目录不存在或不可访问", error))?;
    if !attributes.is_dir() {
        return Err(ApiError::bad_request("目标路径不是目录"));
    }

    let mut planned: Vec<(PathBuf, String, u64)> = Vec::new();
    let mut dirs_to_create: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    for local in &req.local_paths {
        let local_path = PathBuf::from(local);
        let metadata = tokio::fs::metadata(&local_path).await.map_err(|error| {
            ApiError::new(ErrorCode::InvalidPath, "本地路径不存在").detail(error.to_string())
        })?;
        let name = local_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ApiError::bad_request("本地文件名无效"))?
            .to_string();
        let mut remote_base = join_remote(&remote_dir, &name);

        if metadata.is_dir() {
            if sftp.symlink_metadata(&remote_base).await.is_ok() {
                match policy {
                    Policy::Ask => {
                        conflicts.push(remote_base.clone());
                        continue;
                    }
                    Policy::Skip => {
                        skipped.push(remote_base);
                        continue;
                    }
                    Policy::Rename => remote_base = unique_remote_path(&sftp, &remote_base).await?,
                    Policy::Overwrite => {}
                }
            }
            dirs_to_create.push(remote_base.clone());
            for (path, relative, size, is_dir) in walk_local_tree(&local_path)? {
                let remote = join_remote(&remote_base, &relative);
                if is_dir {
                    dirs_to_create.push(remote);
                    continue;
                }
                let mut remote_file = remote;
                if sftp.symlink_metadata(&remote_file).await.is_ok() {
                    match policy {
                        Policy::Ask => {
                            conflicts.push(remote_file);
                            continue;
                        }
                        Policy::Skip => {
                            skipped.push(remote_file);
                            continue;
                        }
                        Policy::Rename => {
                            remote_file = unique_remote_path(&sftp, &remote_file).await?
                        }
                        Policy::Overwrite => {}
                    }
                }
                planned.push((path, remote_file, size));
            }
        } else if metadata.is_file() {
            if sftp.symlink_metadata(&remote_base).await.is_ok() {
                match policy {
                    Policy::Ask => {
                        conflicts.push(remote_base.clone());
                        continue;
                    }
                    Policy::Skip => {
                        skipped.push(remote_base);
                        continue;
                    }
                    Policy::Rename => remote_base = unique_remote_path(&sftp, &remote_base).await?,
                    Policy::Overwrite => {}
                }
            }
            planned.push((local_path, remote_base, metadata.len()));
        } else {
            return Err(ApiError::bad_request("只支持普通文件或目录"));
        }
    }

    if !conflicts.is_empty() {
        return Err(conflict_error(&conflicts));
    }

    // 先建目录（父目录路径更短，排序后创建），再启动文件任务。
    dirs_to_create.sort_by_key(|path| path.matches('/').count());
    for dir in &dirs_to_create {
        let _ = sftp.create_dir(dir).await;
    }

    let mut transfer_ids = Vec::new();
    for (local, remote, size) in planned {
        let transfer_id = new_transfer_id();
        let cancel = state.transfers().register(&transfer_id);
        let transfers = state.transfers().clone();
        let app_handle = app.clone();
        let session_id = req.session_id.clone();
        let slot_clone = slot.clone();
        let handle_clone = handle.clone();
        let preserve_mode = req.preserve_mode;
        let id_clone = transfer_id.clone();
        let remote_clone = remote.clone();

        tauri::async_runtime::spawn(async move {
            // 先发 0 字节进度，确保任务失败（如连接断开）时 UI 仍能看到失败状态。
            emit_progress(
                &app_handle,
                &id_clone,
                &session_id,
                "upload",
                &local,
                &remote_clone,
                0,
                size,
                Instant::now(),
                false,
            );
            let result = run_upload(
                &app_handle,
                &id_clone,
                &session_id,
                &slot_clone,
                handle_clone,
                &local,
                &remote_clone,
                size,
                preserve_mode,
                policy == Policy::Overwrite,
                &cancel,
            )
            .await;
            transfers.remove(&id_clone);
            match result {
                Ok(()) => {
                    events::transfer_done(&app_handle, &id_clone, &session_id, "completed", None)
                }
                Err(error) if error.code == ErrorCode::TransferCancelled => {
                    events::transfer_done(&app_handle, &id_clone, &session_id, "cancelled", None)
                }
                Err(error) => events::transfer_done(
                    &app_handle,
                    &id_clone,
                    &session_id,
                    "failed",
                    Some(error),
                ),
            }
        });

        transfer_ids.push(transfer_id);
    }

    Ok(TransferStartResult {
        transfer_ids,
        skipped,
        local_dir: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_upload(
    app: &AppHandle,
    transfer_id: &str,
    session_id: &str,
    slot: &SftpSlot,
    handle: Arc<russh::client::Handle<ClientHandler>>,
    local: &Path,
    remote: &str,
    size: u64,
    preserve_mode: bool,
    overwrite: bool,
    cancel: &AtomicBool,
) -> ApiResult<()> {
    let sftp = ensure_sftp_from(slot, handle).await?;
    let mut source = tokio::fs::File::open(local).await.map_err(|error| {
        ApiError::new(ErrorCode::InvalidPath, "打开本地文件失败").detail(error.to_string())
    })?;

    let temp = format!(
        "{remote}.nestterm-{}.part",
        &uuid::Uuid::now_v7().simple().to_string()[..8]
    );
    let mut target = sftp
        .create(&temp)
        .await
        .map_err(|error| map_error("创建远端临时文件失败", error))?;

    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut transferred = 0_u64;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    emit_progress(
        app,
        transfer_id,
        session_id,
        "upload",
        local,
        remote,
        0,
        size,
        started,
        false,
    );

    let outcome: ApiResult<()> = async {
        loop {
            if reports_cancelled(cancel) {
                return Err(cancelled_error());
            }
            let read = source.read(&mut buffer).await.map_err(|error| {
                ApiError::new(ErrorCode::TransferIo, "读取本地文件失败").detail(error.to_string())
            })?;
            if read == 0 {
                break;
            }
            target.write_all(&buffer[..read]).await.map_err(|error| {
                ApiError::new(ErrorCode::TransferIo, "写入远端失败").detail(error.to_string())
            })?;
            transferred += read as u64;
            if last_emit.elapsed() >= PROGRESS_INTERVAL {
                emit_progress(
                    app,
                    transfer_id,
                    session_id,
                    "upload",
                    local,
                    remote,
                    transferred,
                    size,
                    started,
                    false,
                );
                last_emit = Instant::now();
            }
        }
        target.flush().await.map_err(|error| {
            ApiError::new(ErrorCode::TransferIo, "刷新远端文件失败").detail(error.to_string())
        })?;
        let _ = target.shutdown().await;

        if preserve_mode {
            if let Ok(metadata) = std::fs::metadata(local) {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = metadata.permissions().mode() & 0o7777;
                    let attributes = FileAttributes {
                        permissions: Some(mode),
                        ..Default::default()
                    };
                    let _ = sftp.set_metadata(&temp, attributes).await;
                }
            }
        }

        // OpenSSH 的 SFTP rename 不覆盖已存在文件：覆盖策略先把旧文件挪到备份，
        // 成功后删除备份；失败则回滚，避免用户在“覆盖”时丢原文件。
        let mut backup: Option<String> = None;
        if overwrite && sftp.symlink_metadata(remote).await.is_ok() {
            let candidate = format!(
                "{remote}.nestterm-old-{}",
                &uuid::Uuid::now_v7().simple().to_string()[..8]
            );
            if sftp.rename(remote, &candidate).await.is_ok() {
                backup = Some(candidate);
            }
        }

        if let Err(error) = sftp.rename(&temp, remote).await {
            if let Some(backup) = &backup {
                let _ = sftp.rename(backup, remote).await;
            }
            return Err(map_error("替换远端文件失败", error));
        }
        if let Some(backup) = &backup {
            let _ = sftp.remove_file(backup).await;
        }
        Ok(())
    }
    .await;

    if let Err(error) = outcome {
        let _ = sftp.remove_file(&temp).await;
        return Err(error);
    }

    emit_progress(
        app,
        transfer_id,
        session_id,
        "upload",
        local,
        remote,
        size,
        size,
        started,
        true,
    );
    Ok(())
}

// ---------------- 下载 ----------------

#[tauri::command]
pub async fn sftp_download(
    app: AppHandle,
    state: State<'_, AppState>,
    req: DownloadRequest,
) -> ApiResult<TransferStartResult> {
    let policy = parse_policy(req.conflict_policy.as_deref())?;
    let slot = state.sessions().sftp_slot(&req.session_id)?;
    let handle = state.sessions().handle(&req.session_id)?;
    let sftp = ensure_sftp_from(&slot, handle.clone()).await?;

    let local_dir = match &req.local_dir {
        Some(dir) => {
            let path = PathBuf::from(dir);
            std::fs::create_dir_all(&path)?;
            path
        }
        None => default_download_dir()?,
    };

    let mut planned: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut dirs_to_create: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    for remote in &req.remote_paths {
        let remote_path = ensure_remote_path(remote)?;
        let metadata = sftp
            .symlink_metadata(&remote_path)
            .await
            .map_err(|error| map_error("读取远端路径失败", error))?;
        let name = base_name(&remote_path);
        let mut local_base = local_dir.join(&name);

        if metadata.is_dir() {
            if local_base.exists() {
                match policy {
                    Policy::Ask => {
                        conflicts.push(local_base.to_string_lossy().to_string());
                        continue;
                    }
                    Policy::Skip => {
                        skipped.push(local_base.to_string_lossy().to_string());
                        continue;
                    }
                    Policy::Rename => local_base = unique_local_path(&local_base)?,
                    Policy::Overwrite => {}
                }
            }
            dirs_to_create.push(local_base.clone());
            for (remote_child, relative, size, is_dir) in
                walk_remote_tree(&sftp, &remote_path).await?
            {
                let local = join_local(&local_base, &relative);
                if is_dir {
                    dirs_to_create.push(local);
                    continue;
                }
                let mut local_file = local;
                if local_file.exists() {
                    match policy {
                        Policy::Ask => {
                            conflicts.push(local_file.to_string_lossy().to_string());
                            continue;
                        }
                        Policy::Skip => {
                            skipped.push(local_file.to_string_lossy().to_string());
                            continue;
                        }
                        Policy::Rename => local_file = unique_local_path(&local_file)?,
                        Policy::Overwrite => {}
                    }
                }
                planned.push((remote_child, local_file, size));
            }
        } else if metadata.is_regular() {
            let mut local = local_base;
            if local.exists() {
                match policy {
                    Policy::Ask => {
                        conflicts.push(local.to_string_lossy().to_string());
                        continue;
                    }
                    Policy::Skip => {
                        skipped.push(local.to_string_lossy().to_string());
                        continue;
                    }
                    Policy::Rename => local = unique_local_path(&local)?,
                    Policy::Overwrite => {}
                }
            }
            planned.push((remote_path, local, metadata.len()));
        } else {
            return Err(ApiError::bad_request("只支持普通文件或目录"));
        }
    }

    if !conflicts.is_empty() {
        return Err(conflict_error(&conflicts));
    }

    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)?;
    }

    let mut transfer_ids = Vec::new();
    for (remote, local, size) in planned {
        let transfer_id = new_transfer_id();
        let cancel = state.transfers().register(&transfer_id);
        let transfers = state.transfers().clone();
        let app_handle = app.clone();
        let session_id = req.session_id.clone();
        let slot_clone = slot.clone();
        let handle_clone = handle.clone();
        let preserve_mode = req.preserve_mode;
        let id_clone = transfer_id.clone();
        let remote_clone = remote.clone();

        tauri::async_runtime::spawn(async move {
            emit_progress(
                &app_handle,
                &id_clone,
                &session_id,
                "download",
                &local,
                &remote_clone,
                0,
                size,
                Instant::now(),
                false,
            );
            let result = run_download(
                &app_handle,
                &id_clone,
                &session_id,
                &slot_clone,
                handle_clone,
                &remote_clone,
                &local,
                size,
                preserve_mode,
                policy == Policy::Overwrite,
                &cancel,
            )
            .await;
            transfers.remove(&id_clone);
            match result {
                Ok(()) => {
                    events::transfer_done(&app_handle, &id_clone, &session_id, "completed", None)
                }
                Err(error) if error.code == ErrorCode::TransferCancelled => {
                    events::transfer_done(&app_handle, &id_clone, &session_id, "cancelled", None)
                }
                Err(error) => events::transfer_done(
                    &app_handle,
                    &id_clone,
                    &session_id,
                    "failed",
                    Some(error),
                ),
            }
        });

        transfer_ids.push(transfer_id);
    }

    Ok(TransferStartResult {
        transfer_ids,
        skipped,
        local_dir: Some(local_dir.to_string_lossy().to_string()),
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_download(
    app: &AppHandle,
    transfer_id: &str,
    session_id: &str,
    slot: &SftpSlot,
    handle: Arc<russh::client::Handle<ClientHandler>>,
    remote: &str,
    local: &Path,
    size: u64,
    preserve_mode: bool,
    overwrite: bool,
    cancel: &AtomicBool,
) -> ApiResult<()> {
    let sftp = ensure_sftp_from(slot, handle).await?;
    let metadata = sftp
        .symlink_metadata(remote)
        .await
        .map_err(|error| map_error("读取远端文件属性失败", error))?;
    let mut source = sftp
        .open(remote)
        .await
        .map_err(|error| map_error("打开远端文件失败", error))?;

    let temp = local.with_file_name(format!(
        "{}.nestterm-{}.part",
        local
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("download"),
        &uuid::Uuid::now_v7().simple().to_string()[..8]
    ));
    let mut target = tokio::fs::File::create(&temp).await.map_err(|error| {
        ApiError::new(ErrorCode::TransferIo, "创建本地临时文件失败").detail(error.to_string())
    })?;

    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut transferred = 0_u64;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    emit_progress(
        app,
        transfer_id,
        session_id,
        "download",
        local,
        remote,
        0,
        size,
        started,
        false,
    );

    let outcome: ApiResult<()> = async {
        loop {
            if reports_cancelled(cancel) {
                return Err(cancelled_error());
            }
            let read = source.read(&mut buffer).await.map_err(|error| {
                ApiError::new(ErrorCode::TransferIo, "读取远端失败").detail(error.to_string())
            })?;
            if read == 0 {
                break;
            }
            target.write_all(&buffer[..read]).await.map_err(|error| {
                ApiError::new(ErrorCode::TransferIo, "写入本地文件失败").detail(error.to_string())
            })?;
            transferred += read as u64;
            if last_emit.elapsed() >= PROGRESS_INTERVAL {
                emit_progress(
                    app,
                    transfer_id,
                    session_id,
                    "download",
                    local,
                    remote,
                    transferred,
                    size,
                    started,
                    false,
                );
                last_emit = Instant::now();
            }
        }
        target.flush().await.map_err(|error| {
            ApiError::new(ErrorCode::TransferIo, "刷新本地文件失败").detail(error.to_string())
        })?;
        let _ = target.shutdown().await;

        if overwrite && local.exists() {
            let _ = tokio::fs::remove_file(local).await;
        }
        tokio::fs::rename(&temp, local).await.map_err(|error| {
            ApiError::new(ErrorCode::TransferIo, "重命名本地文件失败").detail(error.to_string())
        })?;

        if preserve_mode {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = mode_of(&metadata);
                if mode != 0 {
                    let _ = std::fs::set_permissions(local, std::fs::Permissions::from_mode(mode));
                }
            }
        }
        Ok(())
    }
    .await;

    if let Err(error) = outcome {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error);
    }

    emit_progress(
        app,
        transfer_id,
        session_id,
        "download",
        local,
        remote,
        size,
        size,
        started,
        true,
    );
    Ok(())
}

#[tauri::command]
pub fn sftp_cancel_transfer(state: State<'_, AppState>, transfer_id: String) -> ApiResult<()> {
    if state.transfers().cancel(&transfer_id) {
        Ok(())
    } else {
        Err(ApiError::new(ErrorCode::TransferIo, "传输不存在或已经结束"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_policy_parsing() {
        assert_eq!(parse_policy(None).unwrap(), Policy::Ask);
        assert_eq!(parse_policy(Some("ask")).unwrap(), Policy::Ask);
        assert_eq!(parse_policy(Some("overwrite")).unwrap(), Policy::Overwrite);
        assert_eq!(parse_policy(Some("skip")).unwrap(), Policy::Skip);
        assert_eq!(parse_policy(Some("rename")).unwrap(), Policy::Rename);
        assert!(parse_policy(Some("bogus")).is_err());
    }

    #[test]
    fn rename_index_keeps_extension() {
        assert_eq!(with_index("nginx.conf", 1), "nginx (1).conf");
        assert_eq!(with_index("README", 2), "README (2)");
        assert_eq!(with_index(".bashrc", 1), ".bashrc (1)");
    }
}
