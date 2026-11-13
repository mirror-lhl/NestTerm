//! FinalShell 本地配置导入（macOS 路径已实测）。
//!
//! FinalShell 4.6.x 在 macOS 把每个连接存为明文 JSON：
//! `~/Library/FinalShell/conn/<id>_connect_config.json`
//! 字段包括 `name/host/port/user_name/password/description/parent_id/...`，
//! 但 `password` 是 32 字符 base64（加密/不可逆），本项目**不尝试解密**，
//! 导入后首次连接需重新输入密码。
//!
//! 注意：这里只读文本，不执行任何内容；不支持的分组结构会降级为统一导入分组。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::models::{AuthType, ProfileInput};
use crate::db::repo;
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::state::AppState;

const IMPORT_GROUP_NAME: &str = "FinalShell 导入";

#[derive(Debug, Deserialize)]
struct RawConnection {
    #[serde(default)]
    name: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    user_name: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parent_id: String,
    #[serde(default)]
    conection_type: i32,
    #[serde(default)]
    authentication_type: i32,
    #[serde(default)]
    terminal_encoding: String,
}

#[derive(Debug, Clone)]
pub struct ScannedConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub notes: String,
    pub has_password: bool,
    pub auth_type: i32,
    pub charset: String,
    pub parent_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalShellPreview {
    pub dir: String,
    pub connections: usize,
    pub with_password: usize,
    pub groups_detected: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalShellImportResult {
    pub group_id: String,
    pub created: usize,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

/// macOS 已实测；Windows/Linux 给常见候选，仍支持调用方传 `dir` 覆盖。
pub fn default_conn_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let home = PathBuf::from(home);
    #[cfg(target_os = "macos")]
    {
        Some(home.join("Library").join("FinalShell").join("conn"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|dir| dir.join("FinalShell").join("conn"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Some(home.join(".finalshell").join("conn"))
    }
}

fn expand_home(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn resolve_dir(dir: Option<&str>) -> ApiResult<PathBuf> {
    let path = match dir {
        Some(value) if !value.trim().is_empty() => expand_home(value.trim()),
        _ => default_conn_dir()
            .ok_or_else(|| ApiError::new(ErrorCode::ImportParse, "无法定位 FinalShell 数据目录"))?,
    };
    if !path.is_dir() {
        return Err(ApiError::new(
            ErrorCode::ImportParse,
            format!("FinalShell 连接目录不存在：{}", path.display()),
        ));
    }
    Ok(path)
}

fn parse_connection(content: &str) -> Result<Option<ScannedConnection>, String> {
    let raw: RawConnection = serde_json::from_str(content).map_err(|error| error.to_string())?;
    if raw.host.trim().is_empty() || raw.port == 0 {
        return Ok(None);
    }
    // 100 = SSH 普通连接；其它类型（如后续新增的协议）先跳过并告警。
    if raw.conection_type != 0 && raw.conection_type != 100 {
        return Ok(None);
    }
    let name = if raw.name.trim().is_empty() {
        raw.host.trim().to_string()
    } else {
        raw.name.trim().to_string()
    };
    Ok(Some(ScannedConnection {
        name,
        host: raw.host.trim().to_string(),
        port: raw.port,
        username: raw.user_name.trim().to_string(),
        notes: raw.description.trim().to_string(),
        has_password: !raw.password.is_empty(),
        auth_type: raw.authentication_type,
        charset: if raw.terminal_encoding.trim().is_empty() {
            "utf-8".to_string()
        } else {
            raw.terminal_encoding.trim().to_lowercase()
        },
        parent_id: raw.parent_id.trim().to_string(),
    }))
}

fn scan_dir(dir: &Path) -> ApiResult<(Vec<ScannedConnection>, Vec<String>)> {
    let mut connections = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();

    let entries = std::fs::read_dir(dir).map_err(|error| {
        ApiError::new(ErrorCode::ImportParse, "读取 FinalShell 目录失败").detail(error.to_string())
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            ApiError::new(ErrorCode::ImportParse, "遍历 FinalShell 目录失败")
                .detail(error.to_string())
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !name.ends_with("_connect_config.json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                warnings.push(format!("{name}: 读取失败（{error}）"));
                continue;
            }
        };
        match parse_connection(&content) {
            Ok(Some(connection)) => {
                let key = format!(
                    "{}|{}|{}",
                    connection.host, connection.port, connection.username
                );
                if !seen.insert(key) {
                    warnings.push(format!("{}: 与已有连接重复，已跳过", connection.name));
                    continue;
                }
                connections.push(connection);
            }
            Ok(None) => {
                warnings.push(format!("{name}: 不是 SSH 连接或缺少主机，已跳过"));
            }
            Err(error) => {
                warnings.push(format!("{name}: 解析失败（{error}）"));
            }
        }
    }

    connections.sort_by(|left, right| left.name.cmp(&right.name));
    Ok((connections, warnings))
}

#[tauri::command]
pub fn finalshell_preview(
    _state: tauri::State<'_, AppState>,
    dir: Option<String>,
) -> ApiResult<FinalShellPreview> {
    let dir = resolve_dir(dir.as_deref())?;
    let (connections, mut warnings) = scan_dir(&dir)?;
    if connections.is_empty() {
        return Err(ApiError::new(
            ErrorCode::ImportEmpty,
            format!("{} 中没有可导入的 SSH 连接", dir.display()),
        ));
    }
    let with_password = connections.iter().filter(|item| item.has_password).count();
    let groups_detected = connections
        .iter()
        .filter(|item| !item.parent_id.is_empty() && item.parent_id != "root")
        .count();
    if with_password > 0 {
        warnings.push(format!(
            "{with_password} 个连接带有 FinalShell 加密密码，不会导入；首次连接时请重新输入"
        ));
    }
    if groups_detected > 0 {
        warnings.push(format!(
            "检测到 {groups_detected} 个连接引用了分组，但缺少分组元数据；将统一放入「{IMPORT_GROUP_NAME}」"
        ));
    }
    Ok(FinalShellPreview {
        dir: dir.display().to_string(),
        connections: connections.len(),
        with_password,
        groups_detected,
        warnings,
    })
}

#[tauri::command]
pub fn finalshell_import(
    state: tauri::State<'_, AppState>,
    dir: Option<String>,
) -> ApiResult<FinalShellImportResult> {
    let dir = resolve_dir(dir.as_deref())?;
    let (connections, mut warnings) = scan_dir(&dir)?;
    if connections.is_empty() {
        return Err(ApiError::new(
            ErrorCode::ImportEmpty,
            "没有可导入的 SSH 连接",
        ));
    }

    let db = state.db()?;
    let group = db.with(|conn| {
        let groups = repo::groups::list(conn)?;
        match groups
            .into_iter()
            .find(|group| group.name == IMPORT_GROUP_NAME)
        {
            Some(group) => Ok(group),
            None => repo::groups::create(conn, None, IMPORT_GROUP_NAME),
        }
    })?;

    let existing = db.with(|conn| repo::profiles::list(conn, repo::profiles::GroupFilter::All))?;
    let mut created = 0_usize;
    let mut skipped = 0_usize;

    for connection in connections {
        if existing.iter().any(|profile| {
            profile.host == connection.host
                && profile.port == connection.port
                && profile.username == connection.username
        }) {
            skipped += 1;
            warnings.push(format!("{}: 目标中已存在，跳过", connection.name));
            continue;
        }
        if connection.auth_type != 1 {
            warnings.push(format!(
                "{}: FinalShell 认证类型不是密码，已按密码方式导入，请编辑",
                connection.name
            ));
        }
        let mut notes = connection.notes.clone();
        if !notes.is_empty() {
            notes.push_str("\n\n");
        }
        notes.push_str("从 FinalShell 导入；密码未迁移，首次连接需输入。");

        let input = ProfileInput {
            name: connection.name.clone(),
            group_id: Some(group.id.clone()),
            host: connection.host.clone(),
            port: connection.port,
            username: if connection.username.is_empty() {
                "root".to_string()
            } else {
                connection.username.clone()
            },
            auth_type: AuthType::Password,
            key_path: None,
            agent_identity: None,
            term: None,
            charset: Some(connection.charset.clone()),
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: Some(vec!["finalshell".to_string()]),
            color: None,
            notes: Some(notes),
            secret: None,
        };
        match db.with(|conn| repo::profiles::create(conn, &input)) {
            Ok(_) => created += 1,
            Err(error) => warnings.push(format!(
                "{}: 导入失败（{}）",
                connection.name, error.message
            )),
        }
    }

    if created == 0 && skipped == 0 {
        return Err(ApiError::new(ErrorCode::ImportEmpty, "没有导入任何连接"));
    }
    warnings.truncate(50);
    Ok(FinalShellImportResult {
        group_id: group.id,
        created,
        skipped,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "name":"演示连接","host":"192.0.2.10","port":22,"user_name":"demo",
        "password":"QUJDREVGR0hJSktMTU5PUA==","description":"demo note",
        "parent_id":"root","conection_type":100,"authentication_type":1,
        "terminal_encoding":"UTF-8"
    }"#;

    #[test]
    fn parses_plain_connection_without_exposing_password() {
        let parsed = parse_connection(SAMPLE).unwrap().expect("connection");
        assert_eq!(parsed.name, "演示连接");
        assert_eq!(parsed.host, "192.0.2.10");
        assert_eq!(parsed.port, 22);
        assert_eq!(parsed.username, "demo");
        assert_eq!(parsed.charset, "utf-8");
        assert!(parsed.has_password);
        assert_eq!(parsed.parent_id, "root");
    }

    #[test]
    fn skips_non_ssh_and_invalid_entries() {
        let other = r#"{"name":"x","host":"h","port":22,"conection_type":200}"#;
        assert!(parse_connection(other).unwrap().is_none());
        let missing_host = r#"{"name":"x","port":22,"conection_type":100}"#;
        assert!(parse_connection(missing_host).unwrap().is_none());
        assert!(parse_connection("not json").is_err());
    }

    #[test]
    #[ignore = "manual: scan the local FinalShell data directory"]
    fn live_scan_local_finalshell() {
        let dir = default_conn_dir().expect("default dir");
        let (connections, warnings) = scan_dir(&dir).expect("scan");
        println!(
            "dir={} connections={} warnings={}",
            dir.display(),
            connections.len(),
            warnings.len()
        );
        assert!(connections.len() < 10_000);
    }
}
