//! NestTerm 自有 JSON 导出（备份/迁移用）。
//!
//! 原则：
//! - **不导出任何凭据**（password/passphrase/私钥内容都不在 DB，也不会出现）；
//! - 分组用路径表达（`生产/DB`），跨机导入不必依赖 UUID；
//! - 导出文件是普通 JSON，可人工审阅。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::models::{now_ms, AuthType, ConnProfile, Group};
use crate::db::repo;
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportGroup {
    pub path: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProfile {
    pub name: String,
    #[serde(default)]
    pub group_path: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub agent_identity: Option<String>,
    #[serde(default = "default_term")]
    pub term: String,
    #[serde(default = "default_charset")]
    pub charset: String,
    #[serde(default = "default_keepalive")]
    pub keepalive_secs: i64,
    #[serde(default = "default_timeout")]
    pub connect_timeout_secs: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_term() -> String {
    "xterm-256color".to_string()
}

fn default_charset() -> String {
    "utf-8".to_string()
}

fn default_keepalive() -> i64 {
    30
}

fn default_timeout() -> i64 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBundle {
    pub format: String,
    pub version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub credentials_excluded: bool,
    #[serde(default)]
    pub groups: Vec<ExportGroup>,
    #[serde(default)]
    pub profiles: Vec<ExportProfile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub profiles: usize,
    pub groups: usize,
}

pub fn build_group_paths(groups: &[Group]) -> HashMap<String, String> {
    let by_id: HashMap<&str, &Group> = groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect();
    let mut paths: HashMap<String, String> = HashMap::new();

    fn resolve(
        id: &str,
        by_id: &HashMap<&str, &Group>,
        paths: &mut HashMap<String, String>,
        visiting: &mut Vec<String>,
    ) -> String {
        if let Some(path) = paths.get(id) {
            return path.clone();
        }
        if visiting.iter().any(|item| item == id) {
            // 数据损坏时避免无限递归，退化为自身 id。
            return id.to_string();
        }
        visiting.push(id.to_string());
        let path = match by_id.get(id) {
            None => id.to_string(),
            Some(group) => match group.parent_id.as_deref() {
                None => group.name.clone(),
                Some(parent) => {
                    format!("{}/{}", resolve(parent, by_id, paths, visiting), group.name)
                }
            },
        };
        visiting.pop();
        paths.insert(id.to_string(), path.clone());
        path
    }

    for group in groups {
        resolve(&group.id, &by_id, &mut paths, &mut Vec::new());
    }
    paths
}

fn default_export_dir() -> ApiResult<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| ApiError::new(ErrorCode::InvalidPath, "无法定位用户主目录"))?;
    let dir = PathBuf::from(home).join("Downloads").join("NestTerm");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn resolve_export_path(value: Option<&str>) -> ApiResult<PathBuf> {
    let path = match value {
        Some(value) if !value.trim().is_empty() => {
            let value = value.trim();
            if let Some(rest) = value.strip_prefix("~/") {
                let home = std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .ok_or_else(|| ApiError::new(ErrorCode::InvalidPath, "无法定位用户主目录"))?;
                PathBuf::from(home).join(rest)
            } else {
                PathBuf::from(value)
            }
        }
        _ => default_export_dir()?.join(format!("nestterm-export-{}.json", now_ms())),
    };

    let path = if path.is_dir() {
        path.join(format!("nestterm-export-{}.json", now_ms()))
    } else {
        path
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

fn export_profiles(
    profiles: &[ConnProfile],
    paths: &HashMap<String, String>,
) -> Vec<ExportProfile> {
    profiles
        .iter()
        .map(|profile| ExportProfile {
            name: profile.name.clone(),
            group_path: profile
                .group_id
                .as_ref()
                .and_then(|id| paths.get(id).cloned()),
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
            auth_type: profile.auth_type.as_str().to_string(),
            key_path: profile.key_path.clone(),
            agent_identity: profile.agent_identity.clone(),
            term: profile.term.clone(),
            charset: profile.charset.clone(),
            keepalive_secs: profile.keepalive_secs,
            connect_timeout_secs: profile.connect_timeout_secs,
            tags: profile.tags.clone(),
            color: profile.color.clone(),
            notes: profile.notes.clone(),
        })
        .collect()
}

#[tauri::command]
pub fn nestterm_export(
    state: tauri::State<'_, AppState>,
    path: Option<String>,
) -> ApiResult<ExportResult> {
    let db = state.db()?;
    let groups = db.with(repo::groups::list)?;
    let profiles = db.with(|conn| repo::profiles::list(conn, repo::profiles::GroupFilter::All))?;

    let paths = build_group_paths(&groups);
    let mut export_groups: Vec<ExportGroup> = groups
        .iter()
        .map(|group| ExportGroup {
            path: paths
                .get(&group.id)
                .cloned()
                .unwrap_or_else(|| group.name.clone()),
            icon: group.icon.clone(),
            sort_order: group.sort_order,
        })
        .collect();
    export_groups.sort_by(|left, right| left.path.cmp(&right.path));

    let export_profiles = export_profiles(&profiles, &paths);
    let bundle = ExportBundle {
        format: "nestterm-export".to_string(),
        version: 1,
        exported_at: now_ms(),
        credentials_excluded: true,
        groups: export_groups,
        profiles: export_profiles,
    };

    let target = resolve_export_path(path.as_deref())?;
    let json = serde_json::to_string_pretty(&bundle)?;
    std::fs::write(&target, json).map_err(|error| {
        ApiError::new(ErrorCode::InvalidPath, "写入导出文件失败").detail(error.to_string())
    })?;

    Ok(ExportResult {
        path: target.display().to_string(),
        profiles: bundle.profiles.len(),
        groups: bundle.groups.len(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub profiles_created: usize,
    pub groups_created: usize,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

fn expand_input_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn ensure_group_path(
    conn: &rusqlite::Connection,
    path: &str,
    paths: &mut HashMap<String, String>,
    created: &mut usize,
) -> ApiResult<String> {
    let mut parent_id: Option<String> = None;
    let mut current = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        current = if current.is_empty() {
            segment.to_string()
        } else {
            format!("{current}/{segment}")
        };
        if let Some(existing) = paths.get(&current) {
            parent_id = Some(existing.clone());
            continue;
        }
        let group = repo::groups::create(conn, parent_id.as_deref(), segment)?;
        paths.insert(current.clone(), group.id.clone());
        parent_id = Some(group.id);
        *created += 1;
    }
    parent_id.ok_or_else(|| ApiError::bad_request("分组路径为空"))
}

pub fn apply_bundle(db: &crate::db::Db, bundle: &ExportBundle) -> ApiResult<ImportResult> {
    if bundle.format != "nestterm-export" {
        return Err(ApiError::new(
            ErrorCode::ImportVersion,
            "不是 NestTerm 导出文件（format 不匹配）",
        ));
    }
    if bundle.version > 1 {
        return Err(ApiError::new(
            ErrorCode::ImportVersion,
            format!("导出文件版本 {} 高于当前支持版本 1", bundle.version),
        ));
    }

    db.with_mut(|conn| {
        let tx = conn.transaction()?;
        let existing_groups = repo::groups::list(&tx)?;
        let mut paths: HashMap<String, String> = build_group_paths(&existing_groups)
            .into_iter()
            .map(|(id, path)| (path, id))
            .collect();
        let mut groups_created = 0_usize;

        for group in &bundle.groups {
            ensure_group_path(&tx, &group.path, &mut paths, &mut groups_created)?;
        }

        let existing_profiles = repo::profiles::list(&tx, repo::profiles::GroupFilter::All)?;
        let mut profiles_created = 0_usize;
        let mut skipped = 0_usize;
        let mut warnings = Vec::new();

        for profile in &bundle.profiles {
            if existing_profiles.iter().any(|item| {
                item.host == profile.host
                    && item.port == profile.port
                    && item.username == profile.username
            }) {
                skipped += 1;
                continue;
            }
            let group_id = match profile.group_path.as_deref() {
                Some(path) if !path.is_empty() => Some(ensure_group_path(
                    &tx,
                    path,
                    &mut paths,
                    &mut groups_created,
                )?),
                _ => None,
            };
            let auth_type = match profile.auth_type.as_str() {
                "password" => AuthType::Password,
                "key" => AuthType::Key,
                "agent" => AuthType::Agent,
                other => {
                    warnings.push(format!(
                        "{}: 未知认证方式 {}，按密码导入",
                        profile.name, other
                    ));
                    AuthType::Password
                }
            };
            let input = crate::db::models::ProfileInput {
                name: profile.name.clone(),
                group_id,
                host: profile.host.clone(),
                port: profile.port,
                username: profile.username.clone(),
                auth_type,
                key_path: profile.key_path.clone(),
                agent_identity: profile.agent_identity.clone(),
                term: Some(profile.term.clone()),
                charset: Some(profile.charset.clone()),
                keepalive_secs: Some(profile.keepalive_secs),
                connect_timeout_secs: Some(profile.connect_timeout_secs),
                jump_profile_id: None,
                tags: Some(profile.tags.clone()),
                color: profile.color.clone(),
                notes: profile.notes.clone(),
                secret: None,
            };
            match repo::profiles::create(&tx, &input) {
                Ok(_) => profiles_created += 1,
                Err(error) => {
                    warnings.push(format!("{}: 导入失败（{}）", profile.name, error.message))
                }
            }
        }

        tx.commit()?;
        warnings.truncate(50);
        Ok(ImportResult {
            profiles_created,
            groups_created,
            skipped,
            warnings,
        })
    })
}

#[tauri::command]
pub fn nestterm_import(state: tauri::State<'_, AppState>, path: String) -> ApiResult<ImportResult> {
    let target = expand_input_path(path.trim());
    let content = std::fs::read_to_string(&target).map_err(|error| {
        ApiError::new(ErrorCode::ImportParse, "读取导出文件失败").detail(error.to_string())
    })?;
    let bundle: ExportBundle = serde_json::from_str(&content).map_err(|error| {
        ApiError::new(ErrorCode::ImportParse, "JSON 解析失败").detail(error.to_string())
    })?;
    if !bundle.credentials_excluded {
        // 兼容未来可能包含凭据的版本：拒绝，避免把未知字段当配置导入。
        return Err(ApiError::new(
            ErrorCode::ImportVersion,
            "导出文件未声明 credentialsExcluded=true，拒绝导入",
        ));
    }
    apply_bundle(state.db()?, &bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{new_id, Group};

    fn group(id: &str, parent: Option<&str>, name: &str) -> Group {
        Group {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            name: name.into(),
            icon: None,
            sort_order: 0,
            profile_count: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn group_paths_are_nested_and_stable() {
        let groups = vec![
            group("g1", None, "生产"),
            group("g2", Some("g1"), "DB"),
            group("g3", None, "测试"),
        ];
        let paths = build_group_paths(&groups);
        assert_eq!(paths["g1"], "生产");
        assert_eq!(paths["g2"], "生产/DB");
        assert_eq!(paths["g3"], "测试");
        let _ = new_id();
    }

    #[test]
    fn export_profile_never_contains_credentials() {
        let profile = ExportProfile {
            name: "demo".into(),
            group_path: Some("生产".into()),
            host: "192.0.2.10".into(),
            port: 22,
            username: "demo".into(),
            auth_type: "password".into(),
            key_path: None,
            agent_identity: None,
            term: "xterm-256color".into(),
            charset: "utf-8".into(),
            keepalive_secs: 30,
            connect_timeout_secs: 20,
            tags: vec![],
            color: None,
            notes: None,
        };
        let value = serde_json::to_string(&profile).unwrap();
        assert!(!value.contains("\"password\":"));
        assert!(!value.contains("\"secret\":"));
        assert!(!value.contains("\"passphrase\":"));
        assert!(value.contains("192.0.2.10"));
    }

    #[test]
    fn import_is_idempotent_and_never_stores_credentials() {
        use crate::db::{repo, Db};

        let db = Db::open_memory(&[7_u8; 32]).expect("db");
        let bundle = ExportBundle {
            format: "nestterm-export".into(),
            version: 1,
            exported_at: 0,
            credentials_excluded: true,
            groups: vec![ExportGroup {
                path: "运维".into(),
                icon: None,
                sort_order: 0,
            }],
            profiles: vec![ExportProfile {
                name: "demo".into(),
                group_path: Some("运维/DB".into()),
                host: "192.0.2.10".into(),
                port: 22,
                username: "demo".into(),
                auth_type: "password".into(),
                key_path: None,
                agent_identity: None,
                term: "xterm-256color".into(),
                charset: "utf-8".into(),
                keepalive_secs: 30,
                connect_timeout_secs: 20,
                tags: vec!["imported".into()],
                color: None,
                notes: None,
            }],
        };

        let first = apply_bundle(&db, &bundle).expect("first import");
        assert_eq!(first.profiles_created, 1);
        assert_eq!(first.groups_created, 2, "运维 + 运维/DB");
        assert_eq!(first.skipped, 0);

        let second = apply_bundle(&db, &bundle).expect("second import");
        assert_eq!(second.profiles_created, 0);
        assert_eq!(second.skipped, 1);

        let profiles = db
            .with(|conn| repo::profiles::list(conn, repo::profiles::GroupFilter::All))
            .unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(!profiles[0].has_password);
        assert_eq!(profiles[0].host, "192.0.2.10");
        assert_eq!(
            db.with(repo::groups::list).unwrap().len(),
            2,
            "分组不应重复创建"
        );
    }

    #[test]
    fn import_rejects_foreign_or_credentialed_files() {
        use crate::db::Db;

        let db = Db::open_memory(&[7_u8; 32]).expect("db");
        let mut bundle = ExportBundle {
            format: "other-export".into(),
            version: 1,
            exported_at: 0,
            credentials_excluded: true,
            groups: vec![],
            profiles: vec![],
        };
        assert!(apply_bundle(&db, &bundle).is_err());

        bundle.format = "nestterm-export".into();
        bundle.credentials_excluded = false;
        // apply_bundle 只校验 format/version；credentials 由命令层校验。
        assert!(apply_bundle(&db, &bundle).is_ok());
    }
}
