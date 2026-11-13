//! 应用自有 known_hosts：TOFU 记录 + 指纹变更拒绝。
//!
//! 不读写用户 `~/.ssh/known_hosts`（A 期为过渡实现，B 期起以本表为准）。

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::models::{new_id, now_ms, KnownHost};
use crate::error::{ApiError, ApiResult, ErrorCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// 与已有记录一致。
    Trusted,
    /// 首次见到该主机，已按 TOFU 写入。
    Learned,
}

pub fn verify(
    conn: &Connection,
    host: &str,
    port: u16,
    key_type: &str,
    fingerprint: &str,
    public_key: &str,
    strict: bool,
) -> ApiResult<VerifyOutcome> {
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT id, fingerprint FROM known_hosts
             WHERE host = ?1 AND port = ?2 AND key_type = ?3",
            params![host, port, key_type],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    if let Some((id, recorded)) = existing {
        if recorded != fingerprint {
            return Err(ApiError::new(
                ErrorCode::SshHostkeyMismatch,
                "主机指纹与记录不一致，连接已阻断（可能是中间人攻击或服务器重装）；如确认是重装，请在「指纹」中删除旧记录后重连",
            )
            .detail(format!(
                "已记录：{recorded}\n服务端：{fingerprint}\n主机：{host}:{port} ({key_type})"
            )));
        }
        conn.execute(
            "UPDATE known_hosts SET last_seen_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )?;
        return Ok(VerifyOutcome::Trusted);
    }

    if strict {
        return Err(ApiError::new(
            ErrorCode::SshHostkeyUnknown,
            "首次连接该主机（严格模式）。请先确认指纹并信任后再连接",
        )
        .detail(format!("{host}:{port} {key_type} {fingerprint}")));
    }

    let now = now_ms();
    conn.execute(
        "INSERT INTO known_hosts
            (id, host, port, key_type, fingerprint, public_key, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![new_id(), host, port, key_type, fingerprint, public_key, now],
    )?;
    Ok(VerifyOutcome::Learned)
}

pub fn list(conn: &Connection) -> ApiResult<Vec<KnownHost>> {
    let mut stmt = conn.prepare(
        "SELECT id, host, port, key_type, fingerprint, first_seen_at, last_seen_at
         FROM known_hosts ORDER BY host ASC, port ASC, key_type ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(KnownHost {
            id: row.get(0)?,
            host: row.get(1)?,
            port: row.get(2)?,
            key_type: row.get(3)?,
            fingerprint: row.get(4)?,
            first_seen_at: row.get(5)?,
            last_seen_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn remove(conn: &Connection, id: &str) -> ApiResult<()> {
    conn.execute("DELETE FROM known_hosts WHERE id = ?1", params![id])?;
    Ok(())
}
