//! 连接配置仓储。

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use crate::db::models::{new_id, now_ms, AuthType, ConnProfile, ProfileInput};
use crate::error::{ApiError, ApiResult, ErrorCode};

const SELECT_PROFILE: &str = "SELECT id, group_id, name, host, port, username, auth_type, key_path,
        key_fingerprint, agent_identity, has_password, has_passphrase, term, charset,
        keepalive_secs, connect_timeout_secs, jump_profile_id, tags, color, notes,
        sort_order, last_connected_at, created_at, updated_at
     FROM profiles";

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnProfile> {
    let auth_text: String = row.get(6)?;
    let tags_text: String = row.get(17)?;
    Ok(ConnProfile {
        id: row.get(0)?,
        group_id: row.get(1)?,
        name: row.get(2)?,
        host: row.get(3)?,
        port: row.get(4)?,
        username: row.get(5)?,
        auth_type: AuthType::parse(&auth_text).unwrap_or(AuthType::Password),
        key_path: row.get(7)?,
        key_fingerprint: row.get(8)?,
        agent_identity: row.get(9)?,
        has_password: row.get::<_, i64>(10)? != 0,
        has_passphrase: row.get::<_, i64>(11)? != 0,
        term: row.get(12)?,
        charset: row.get(13)?,
        keepalive_secs: row.get(14)?,
        connect_timeout_secs: row.get(15)?,
        jump_profile_id: row.get(16)?,
        tags: serde_json::from_str(&tags_text).unwrap_or_default(),
        color: row.get(18)?,
        notes: row.get(19)?,
        sort_order: row.get(20)?,
        last_connected_at: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

#[allow(dead_code)]
pub enum GroupFilter<'a> {
    All,
    Ungrouped,
    Group(&'a str),
}

pub fn list(conn: &Connection, filter: GroupFilter<'_>) -> ApiResult<Vec<ConnProfile>> {
    let (sql, args): (String, Vec<String>) = match filter {
        GroupFilter::All => (
            format!("{SELECT_PROFILE} ORDER BY sort_order ASC, name ASC"),
            vec![],
        ),
        GroupFilter::Ungrouped => (
            format!("{SELECT_PROFILE} WHERE group_id IS NULL ORDER BY sort_order ASC, name ASC"),
            vec![],
        ),
        GroupFilter::Group(id) => (
            format!("{SELECT_PROFILE} WHERE group_id = ?1 ORDER BY sort_order ASC, name ASC"),
            vec![id.to_string()],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), row_to_profile)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get(conn: &Connection, id: &str) -> ApiResult<ConnProfile> {
    conn.query_row(
        &format!("{SELECT_PROFILE} WHERE id = ?1"),
        params![id],
        row_to_profile,
    )
    .optional()?
    .ok_or_else(|| ApiError::new(ErrorCode::ProfileNotFound, "连接配置不存在"))
}

fn ensure_group(conn: &Connection, group_id: Option<&str>) -> ApiResult<()> {
    if let Some(id) = group_id {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM groups WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Err(ApiError::new(ErrorCode::GroupNotFound, "目标分组不存在"));
        }
    }
    Ok(())
}

fn next_sort_order(conn: &Connection, group_id: Option<&str>) -> ApiResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM profiles WHERE group_id IS ?1",
        params![group_id],
        |row| row.get(0),
    )?)
}

pub fn create(conn: &Connection, input: &ProfileInput) -> ApiResult<ConnProfile> {
    input.validate()?;
    ensure_group(conn, input.group_id.as_deref())?;

    let id = new_id();
    let now = now_ms();
    let sort_order = next_sort_order(conn, input.group_id.as_deref())?;
    let tags = input.tags_json()?;

    conn.execute(
        "INSERT INTO profiles (
            id, group_id, name, host, port, username, auth_type, key_path, agent_identity,
            term, charset, keepalive_secs, connect_timeout_secs, jump_profile_id,
            tags, color, notes, sort_order, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
            ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?19
         )",
        params![
            id,
            input.group_id,
            input.name.trim(),
            input.host.trim(),
            input.port as i64,
            input.username.trim(),
            input.auth_type.as_str(),
            input.key_path,
            input.agent_identity,
            input.term.as_deref().unwrap_or("xterm-256color"),
            input.charset.as_deref().unwrap_or("utf-8"),
            input.keepalive_secs.unwrap_or(30),
            input.connect_timeout_secs.unwrap_or(20),
            input.jump_profile_id,
            tags,
            input.color,
            input.notes,
            sort_order,
            now,
        ],
    )?;

    get(conn, &id)
}

/// B1 采用全量更新（前端始终持有完整记录）；新增凭据用 `secret`，删除用 `profile_clear_secret`。
pub fn update(conn: &Connection, id: &str, input: &ProfileInput) -> ApiResult<ConnProfile> {
    let existing = get(conn, id)?;
    input.validate()?;
    ensure_group(conn, input.group_id.as_deref())?;

    let tags = input.tags_json()?;
    let keep_fingerprint = input.auth_type == AuthType::Key
        && existing.auth_type == AuthType::Key
        && existing.key_path == input.key_path;

    let changed = conn.execute(
        "UPDATE profiles SET
            group_id = ?2, name = ?3, host = ?4, port = ?5, username = ?6,
            auth_type = ?7, key_path = ?8, key_fingerprint = ?9, agent_identity = ?10,
            term = ?11, charset = ?12, keepalive_secs = ?13, connect_timeout_secs = ?14,
            jump_profile_id = ?15, tags = ?16, color = ?17, notes = ?18, updated_at = ?19
         WHERE id = ?1",
        params![
            id,
            input.group_id,
            input.name.trim(),
            input.host.trim(),
            input.port as i64,
            input.username.trim(),
            input.auth_type.as_str(),
            input.key_path,
            if keep_fingerprint {
                existing.key_fingerprint.clone()
            } else {
                None
            },
            input.agent_identity,
            input.term.as_deref().unwrap_or(existing.term.as_str()),
            input
                .charset
                .as_deref()
                .unwrap_or(existing.charset.as_str()),
            input.keepalive_secs.unwrap_or(existing.keepalive_secs),
            input
                .connect_timeout_secs
                .unwrap_or(existing.connect_timeout_secs),
            input.jump_profile_id,
            tags,
            input.color,
            input.notes,
            now_ms(),
        ],
    )?;

    if changed == 0 {
        return Err(ApiError::new(ErrorCode::ProfileNotFound, "连接配置不存在"));
    }
    get(conn, id)
}

pub fn delete_many(conn: &mut Connection, ids: &[String]) -> ApiResult<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let tx = conn.transaction()?;
    let deleted = tx.execute(
        &format!("DELETE FROM profiles WHERE id IN ({placeholders})"),
        params_from_iter(ids.iter()),
    )?;
    tx.commit()?;
    Ok(deleted as i64)
}

pub fn duplicate(conn: &Connection, id: &str, name: Option<&str>) -> ApiResult<ConnProfile> {
    let source = get(conn, id)?;
    let input = ProfileInput {
        name: name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("{} 副本", source.name)),
        group_id: source.group_id.clone(),
        host: source.host.clone(),
        port: source.port,
        username: source.username.clone(),
        auth_type: source.auth_type,
        key_path: source.key_path.clone(),
        agent_identity: source.agent_identity.clone(),
        term: Some(source.term.clone()),
        charset: Some(source.charset.clone()),
        keepalive_secs: Some(source.keepalive_secs),
        connect_timeout_secs: Some(source.connect_timeout_secs),
        jump_profile_id: source.jump_profile_id.clone(),
        tags: Some(source.tags.clone()),
        color: source.color.clone(),
        notes: source.notes.clone(),
        // 复制配置不复制凭据：新记录必须重新输入密码/口令。
        secret: None,
    };
    create(conn, &input)
}

pub fn move_profile(
    conn: &mut Connection,
    id: &str,
    group_id: Option<&str>,
    index: usize,
) -> ApiResult<()> {
    let _ = get(conn, id)?;
    ensure_group(conn, group_id)?;

    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE profiles SET group_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, group_id, now_ms()],
    )?;

    let mut siblings: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM profiles WHERE group_id IS ?1 AND id <> ?2
             ORDER BY sort_order ASC, name ASC",
        )?;
        let rows = stmt.query_map(params![group_id, id], |row| row.get::<_, String>(0))?;
        let collected = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        collected
    };
    let at = index.min(siblings.len());
    siblings.insert(at, id.to_string());
    for (position, sibling) in siblings.iter().enumerate() {
        tx.execute(
            "UPDATE profiles SET sort_order = ?2 WHERE id = ?1",
            params![sibling, (position as i64 + 1) * 1000],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn touch_last_connected(conn: &Connection, id: &str, timestamp: i64) -> ApiResult<()> {
    conn.execute(
        "UPDATE profiles SET last_connected_at = ?2 WHERE id = ?1",
        params![id, timestamp],
    )?;
    Ok(())
}

pub fn set_secret_flag(conn: &Connection, id: &str, kind: &str, present: bool) -> ApiResult<()> {
    let column = match kind {
        "password" => "has_password",
        "passphrase" => "has_passphrase",
        _ => return Err(ApiError::bad_request("未知的凭据类型")),
    };
    let changed = conn.execute(
        &format!("UPDATE profiles SET {column} = ?2, updated_at = ?3 WHERE id = ?1"),
        params![id, if present { 1 } else { 0 }, now_ms()],
    )?;
    if changed == 0 {
        return Err(ApiError::new(ErrorCode::ProfileNotFound, "连接配置不存在"));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn set_key_fingerprint(
    conn: &Connection,
    id: &str,
    fingerprint: Option<&str>,
) -> ApiResult<()> {
    conn.execute(
        "UPDATE profiles SET key_fingerprint = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, fingerprint, now_ms()],
    )?;
    Ok(())
}
