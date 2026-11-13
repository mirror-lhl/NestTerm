//! 设置仓储：KV + JSON，section 级覆盖。

use rusqlite::{params, Connection};

use crate::db::models::{now_ms, Settings, SettingsPatch};
use crate::error::ApiResult;

pub fn get(conn: &Connection) -> ApiResult<Settings> {
    let mut settings = Settings::default();
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (key, value) = row?;
        match key.as_str() {
            "theme" => {
                if let Ok(theme) = serde_json::from_str::<String>(&value) {
                    settings.theme = theme;
                }
            }
            "terminal" => settings.terminal = serde_json::from_str(&value)?,
            "behavior" => settings.behavior = serde_json::from_str(&value)?,
            "sftp" => settings.sftp = serde_json::from_str(&value)?,
            "monitor" => settings.monitor = serde_json::from_str(&value)?,
            "security" => settings.security = serde_json::from_str(&value)?,
            "ui" => settings.ui = serde_json::from_str(&value)?,
            _ => {}
        }
    }

    Ok(settings)
}

pub fn update(conn: &mut Connection, patch: &SettingsPatch) -> ApiResult<Settings> {
    let tx = conn.transaction()?;
    if let Some(theme) = &patch.theme {
        upsert(&tx, "theme", &serde_json::to_string(theme)?)?;
    }
    if let Some(value) = &patch.terminal {
        upsert(&tx, "terminal", &serde_json::to_string(value)?)?;
    }
    if let Some(value) = &patch.behavior {
        upsert(&tx, "behavior", &serde_json::to_string(value)?)?;
    }
    if let Some(value) = &patch.sftp {
        upsert(&tx, "sftp", &serde_json::to_string(value)?)?;
    }
    if let Some(value) = &patch.monitor {
        upsert(&tx, "monitor", &serde_json::to_string(value)?)?;
    }
    if let Some(value) = &patch.security {
        upsert(&tx, "security", &serde_json::to_string(value)?)?;
    }
    if let Some(value) = &patch.ui {
        upsert(&tx, "ui", &serde_json::to_string(value)?)?;
    }
    tx.commit()?;
    get(conn)
}

fn upsert(conn: &Connection, key: &str, value: &str) -> ApiResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_ms()],
    )?;
    Ok(())
}

pub fn get_raw(conn: &Connection, key: &str) -> ApiResult<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?)
}

pub fn set_raw(conn: &Connection, key: &str, value: &str) -> ApiResult<()> {
    upsert(conn, key, value)
}
