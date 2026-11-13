//! 分组仓储。层级 ≤ 3，同级重名由数据库唯一索引兜底。

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

use crate::db::models::{new_id, now_ms, Group};
use crate::error::{ApiError, ApiResult, ErrorCode};

const SELECT_GROUP: &str = "SELECT g.id, g.parent_id, g.name, g.icon, g.sort_order,
        (SELECT COUNT(*) FROM profiles p WHERE p.group_id = g.id) AS profile_count,
        g.created_at, g.updated_at
     FROM groups g";

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<Group> {
    Ok(Group {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        name: row.get(2)?,
        icon: row.get(3)?,
        sort_order: row.get(4)?,
        profile_count: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub fn get(conn: &Connection, id: &str) -> ApiResult<Group> {
    conn.query_row(
        &format!("{SELECT_GROUP} WHERE g.id = ?1"),
        params![id],
        row_to_group,
    )
    .optional()?
    .ok_or_else(|| ApiError::new(ErrorCode::GroupNotFound, "分组不存在"))
}

pub fn list(conn: &Connection) -> ApiResult<Vec<Group>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_GROUP} ORDER BY g.sort_order ASC, g.name ASC"
    ))?;
    let rows = stmt.query_map([], row_to_group)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn validate_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request("分组名称需为 1–64 个字符"));
    }
    Ok(())
}

fn parent_depth(conn: &Connection, parent_id: &str) -> ApiResult<i64> {
    let depth: i64 = conn.query_row(
        "WITH RECURSIVE anc(id, parent_id, depth) AS (
            SELECT id, parent_id, 1 FROM groups WHERE id = ?1
            UNION ALL
            SELECT g.id, g.parent_id, anc.depth + 1
            FROM groups g JOIN anc ON g.id = anc.parent_id
         )
         SELECT COALESCE(MAX(depth), 0) FROM anc",
        params![parent_id],
        |row| row.get(0),
    )?;
    Ok(depth)
}

pub fn create(conn: &Connection, parent_id: Option<&str>, name: &str) -> ApiResult<Group> {
    let name = name.trim();
    validate_name(name)?;

    if let Some(parent) = parent_id {
        let depth = parent_depth(conn, parent)?;
        if depth == 0 {
            return Err(ApiError::new(ErrorCode::GroupNotFound, "父分组不存在"));
        }
        if depth >= 3 {
            return Err(ApiError::bad_request("分组层级最多 3 级"));
        }
    }

    let sort_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM groups WHERE parent_id IS ?1",
        params![parent_id],
        |row| row.get(0),
    )?;

    let id = new_id();
    let now = now_ms();
    conn.execute(
        "INSERT INTO groups (id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id, parent_id, name, sort_order, now],
    )?;

    get(conn, &id)
}

pub fn rename(conn: &Connection, id: &str, name: &str) -> ApiResult<Group> {
    let name = name.trim();
    validate_name(name)?;
    let changed = conn.execute(
        "UPDATE groups SET name = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, name, now_ms()],
    )?;
    if changed == 0 {
        return Err(ApiError::new(ErrorCode::GroupNotFound, "分组不存在"));
    }
    get(conn, id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteStrategy {
    MoveToParent,
    DeleteAll,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDeleteResult {
    pub deleted_groups: i64,
    pub moved_profiles: i64,
    pub deleted_profiles: i64,
}

pub fn delete(
    conn: &mut Connection,
    id: &str,
    strategy: DeleteStrategy,
) -> ApiResult<GroupDeleteResult> {
    let group = get(conn, id)?;

    let mut subtree_stmt = conn.prepare(
        "WITH RECURSIVE sub(id) AS (
            SELECT ?1
            UNION ALL
            SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id
         )
         SELECT id FROM sub",
    )?;
    let subtree: Vec<String> = subtree_stmt
        .query_map(params![id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(subtree_stmt);

    let tx = conn.transaction()?;
    let result = match strategy {
        DeleteStrategy::MoveToParent => {
            tx.execute(
                "UPDATE groups SET parent_id = ?2, updated_at = ?3 WHERE parent_id = ?1",
                params![id, group.parent_id, now_ms()],
            )?;
            let moved_profiles = tx.execute(
                "UPDATE profiles SET group_id = ?2, updated_at = ?3 WHERE group_id = ?1",
                params![id, group.parent_id, now_ms()],
            )?;
            tx.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
            GroupDeleteResult {
                deleted_groups: 1,
                moved_profiles: moved_profiles as i64,
                deleted_profiles: 0,
            }
        }
        DeleteStrategy::DeleteAll => {
            let placeholders = std::iter::repeat_n("?", subtree.len())
                .collect::<Vec<_>>()
                .join(",");
            let deleted_profiles = tx.execute(
                &format!("DELETE FROM profiles WHERE group_id IN ({placeholders})"),
                params_from_iter(subtree.iter()),
            )?;
            tx.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
            GroupDeleteResult {
                deleted_groups: subtree.len() as i64,
                moved_profiles: 0,
                deleted_profiles: deleted_profiles as i64,
            }
        }
    };
    tx.commit()?;
    Ok(result)
}

/// 移动分组到新的父级，并按 index 重排同级顺序。
pub fn move_group(
    conn: &mut Connection,
    id: &str,
    parent_id: Option<&str>,
    index: usize,
) -> ApiResult<()> {
    let _ = get(conn, id)?;

    if let Some(parent) = parent_id {
        if parent == id {
            return Err(ApiError::new(
                ErrorCode::GroupCycle,
                "分组不能移动到自身之下",
            ));
        }
        let is_descendant: i64 = conn.query_row(
            "WITH RECURSIVE sub(id) AS (
                SELECT ?1
                UNION ALL
                SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id
             )
             SELECT COUNT(*) FROM sub WHERE id = ?2",
            params![id, parent],
            |row| row.get(0),
        )?;
        if is_descendant > 0 {
            return Err(ApiError::new(
                ErrorCode::GroupCycle,
                "分组不能移动到自己的子分组之下",
            ));
        }
        let depth = parent_depth(conn, parent)?;
        if depth == 0 {
            return Err(ApiError::new(ErrorCode::GroupNotFound, "目标分组不存在"));
        }
    }

    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE groups SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, parent_id, now_ms()],
    )?;

    let mut siblings: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM groups WHERE parent_id IS ?1 AND id <> ?2
             ORDER BY sort_order ASC, name ASC",
        )?;
        let rows = stmt.query_map(params![parent_id, id], |row| row.get::<_, String>(0))?;
        let collected = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        collected
    };
    let at = index.min(siblings.len());
    siblings.insert(at, id.to_string());
    for (position, sibling) in siblings.iter().enumerate() {
        tx.execute(
            "UPDATE groups SET sort_order = ?2 WHERE id = ?1",
            params![sibling, (position as i64 + 1) * 1000],
        )?;
    }
    tx.commit()?;
    Ok(())
}
