//! 端口转发规则（表结构 v1 已存在）。

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::db::models::{new_id, now_ms};
use crate::error::{ApiError, ApiResult, ErrorCode};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRule {
    pub id: String,
    pub profile_id: String,
    pub name: String,
    pub kind: String,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub auto_start: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRuleInput {
    pub profile_id: String,
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_bind_host")]
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    #[serde(default)]
    pub auto_start: bool,
}

fn default_kind() -> String {
    "local".to_string()
}

fn default_bind_host() -> String {
    "127.0.0.1".to_string()
}

impl ForwardRuleInput {
    fn validate(&self) -> ApiResult<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            return Err(ApiError::bad_request("规则名称需为 1–64 个字符"));
        }
        if !matches!(self.kind.as_str(), "local" | "dynamic" | "remote") {
            return Err(ApiError::bad_request(
                "kind 只能是 local / remote / dynamic",
            ));
        }
        if self.bind_host.trim().is_empty() {
            return Err(ApiError::bad_request("监听地址不能为空"));
        }
        if self.bind_port == 0 {
            return Err(ApiError::bad_request("监听端口必须大于 0"));
        }
        if self.kind == "local" || self.kind == "remote" {
            if self.target_host.as_deref().unwrap_or("").trim().is_empty() {
                return Err(ApiError::bad_request("本地转发必须填写目标主机"));
            }
            if self.target_port.unwrap_or(0) == 0 {
                return Err(ApiError::bad_request("本地转发必须填写目标端口"));
            }
        }
        Ok(())
    }
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<ForwardRule> {
    Ok(ForwardRule {
        id: row.get(0)?,
        profile_id: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        bind_host: row.get(4)?,
        bind_port: row.get(5)?,
        target_host: row.get(6)?,
        target_port: row.get(7)?,
        auto_start: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const SELECT: &str = "SELECT id, profile_id, name, kind, bind_host, bind_port, target_host, target_port, auto_start, created_at, updated_at FROM forward_rules";

pub fn list(conn: &Connection, profile_id: &str) -> ApiResult<Vec<ForwardRule>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE profile_id = ? ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([profile_id], from_row)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn get(conn: &Connection, id: &str) -> ApiResult<ForwardRule> {
    conn.query_row(&format!("{SELECT} WHERE id = ?"), [id], from_row)
        .optional()?
        .ok_or_else(|| ApiError::new(ErrorCode::SessionNotFound, "转发规则不存在"))
}

pub fn create(conn: &Connection, input: &ForwardRuleInput) -> ApiResult<ForwardRule> {
    input.validate()?;
    let id = new_id();
    let now = now_ms();
    let bind_host = input.bind_host.trim();
    let target_host = input
        .target_host
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    conn.execute(
        "INSERT INTO forward_rules (id, profile_id, name, kind, bind_host, bind_port, target_host, target_port, auto_start, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id, input.profile_id, input.name.trim(), input.kind, bind_host, input.bind_port,
            target_host, input.target_port, if input.auto_start { 1 } else { 0 }, now, now
        ],
    )
    .map_err(|error| {
        if error.to_string().contains("UNIQUE") {
            ApiError::new(ErrorCode::ForwardBind, "该连接已有同地址/端口的本地监听规则")
        } else {
            ApiError::from(error)
        }
    })?;
    get(conn, &id)
}

pub fn delete(conn: &Connection, id: &str) -> ApiResult<()> {
    let changed = conn.execute("DELETE FROM forward_rules WHERE id = ?", [id])?;
    if changed == 0 {
        return Err(ApiError::new(ErrorCode::SessionNotFound, "转发规则不存在"));
    }
    Ok(())
}
