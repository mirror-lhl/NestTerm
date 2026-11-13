//! SQLCipher 数据库：打开、迁移、连接句柄。
//!
//! 设计对应 `docs/03-database-schema.md`：
//! - 整库加密，主密钥来自系统钥匙串；
//! - 单写连接（`Mutex<Connection>`），临界区不跨 `.await`；
//! - `PRAGMA user_version` 做迁移版本，迁移前自动备份。

pub mod models;
pub mod repo;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::{ApiError, ApiResult, ErrorCode};

/// 当前 schema 版本，必须与 `migrations/` 中最大编号一致。
pub const SCHEMA_VERSION: i32 = 1;

const MIGRATIONS: &[(i32, &str)] = &[(1, include_str!("../../migrations/0001_init.sql"))];

#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
    version: i32,
    path: Option<PathBuf>,
}

impl Db {
    pub fn open_file(path: &Path, key: &[u8]) -> ApiResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            if let Some(parent) = path.parent() {
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        Self::open_connection(connection, key, Some(path.to_path_buf()))
    }

    #[allow(dead_code)] // 单测/后续内存模式使用
    pub fn open_memory(key: &[u8]) -> ApiResult<Self> {
        Self::open_connection(Connection::open_in_memory()?, key, None)
    }

    fn open_connection(
        mut connection: Connection,
        key: &[u8],
        path: Option<PathBuf>,
    ) -> ApiResult<Self> {
        if key.len() != 32 {
            return Err(ApiError::internal("SQLCipher 密钥必须为 32 字节"));
        }

        // SQLCipher 要求 key 是连接上的第一条语句；用 x'hex' 原始密钥形式，避免 KDF 差异。
        let pragma_key = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex::encode(key)));
        connection.execute_batch(&pragma_key)?;
        let _ = connection.execute_batch("PRAGMA cipher_memory_security = ON;");

        // 用一次真实读取验证密钥：密钥错误时 SQLite 会报 "file is not a database"。
        connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| {
                ApiError::new(ErrorCode::DbError, "数据库密钥不正确，或数据库文件已损坏")
                    .detail(error.to_string())
            })?;

        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;

        if path.is_some() {
            connection.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;",
            )?;
        }

        let current: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            return Err(ApiError::new(
                ErrorCode::DbMigration,
                "数据库由更新版本的 NestTerm 创建，请升级应用后再打开",
            ));
        }

        if current > 0 && current < SCHEMA_VERSION {
            if let Some(file) = path.as_deref() {
                let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                backup_before_migration(file)?;
            }
        }

        let version = migrate(&mut connection)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(connection)),
            version,
            path,
        })
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 只读/单语句读写。闭包内的 `rusqlite::Error` 会经 `?` 转成 `ApiError`。
    pub fn with<T>(&self, action: impl FnOnce(&Connection) -> ApiResult<T>) -> ApiResult<T> {
        let guard = self.lock();
        action(&guard)
    }

    /// 需要事务的写操作。
    pub fn with_mut<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> ApiResult<T>,
    ) -> ApiResult<T> {
        let mut guard = self.lock();
        action(&mut guard)
    }
}

fn migrate(connection: &mut Connection) -> ApiResult<i32> {
    let mut current: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (version, sql) in MIGRATIONS {
        if *version > current {
            let tx = connection.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", *version)?;
            tx.commit()?;
            current = *version;
        }
    }

    Ok(current)
}

/// 迁移前备份：`nestterm.db.bak.1` / `.bak.2` 滚动保留。
fn backup_before_migration(file: &Path) -> ApiResult<()> {
    let text = file.to_string_lossy();
    let first = PathBuf::from(format!("{text}.bak.1"));
    let second = PathBuf::from(format!("{text}.bak.2"));

    if first.exists() {
        let _ = std::fs::remove_file(&second);
        std::fs::rename(&first, &second)?;
    }
    std::fs::copy(file, &first)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{new_id, now_ms};
    use crate::error::ErrorCode;

    fn key() -> Vec<u8> {
        vec![7u8; 32]
    }

    #[test]
    fn migrate_fresh_memory_db() {
        let db = Db::open_memory(&key()).expect("open");
        assert_eq!(db.version(), 1);
        db.with(|conn| {
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )?;
            assert!(count >= 5);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rejects_wrong_key_on_file_db() {
        let path = std::env::temp_dir().join(format!("nestterm-wrongkey-{}.db", new_id()));
        {
            let _db = Db::open_file(&path, &key()).expect("create");
        }
        let error = match Db::open_file(&path, &[9u8; 32]) {
            Ok(_) => panic!("wrong key must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::DbError);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.bak.1", path.to_string_lossy()));
    }

    #[test]
    fn schema_constraints_are_enforced() {
        let db = Db::open_memory(&key()).expect("open");
        db.with_mut(|conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO groups (id,parent_id,name,created_at,updated_at) VALUES ('g1',NULL,'prod',?,?)",
                rusqlite::params![now, now],
            )?;
            let duplicate = conn.execute(
                "INSERT INTO groups (id,parent_id,name,created_at,updated_at) VALUES ('g2',NULL,'prod',?,?)",
                rusqlite::params![now, now],
            );
            assert!(duplicate.is_err(), "同级重名必须被拒绝");

            let bad_key = conn.execute(
                "INSERT INTO profiles (id,name,host,port,username,auth_type,created_at,updated_at)
                 VALUES ('p1','bad','h',22,'u','key',?,?)",
                rusqlite::params![now, now],
            );
            assert!(bad_key.is_err(), "key 认证缺 key_path 必须被拒绝");

            let bad_tags = conn.execute(
                "INSERT INTO profiles (id,name,host,port,username,auth_type,tags,created_at,updated_at)
                 VALUES ('p2','bad','h',22,'u','agent','{\"a\":1}',?,?)",
                rusqlite::params![now, now],
            );
            assert!(bad_tags.is_err(), "tags 必须是 JSON 数组");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn group_and_profile_crud() {
        use crate::db::models::{AuthType, ProfileInput};
        use crate::db::repo::{groups, profiles};

        let db = Db::open_memory(&key()).expect("open");

        let group = db.with(|conn| groups::create(conn, None, "生产")).unwrap();
        let child = db
            .with(|conn| groups::create(conn, Some(&group.id), "华东"))
            .unwrap();
        assert_eq!(
            db.with(|conn| groups::list(conn)).unwrap().len(),
            2,
            "分组列表应包含父子"
        );

        let input = ProfileInput {
            name: "db-01".into(),
            group_id: Some(child.id.clone()),
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            auth_type: AuthType::Password,
            key_path: None,
            agent_identity: None,
            term: None,
            charset: None,
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: Some(vec!["prod".into(), "db".into()]),
            color: None,
            notes: Some("备注".into()),
            secret: None,
        };
        let profile = db.with(|conn| profiles::create(conn, &input)).unwrap();
        assert_eq!(profile.tags, vec!["prod", "db"]);
        assert!(!profile.has_password);

        db.with(|conn| profiles::set_secret_flag(conn, &profile.id, "password", true))
            .unwrap();
        assert!(
            db.with(|conn| profiles::get(conn, &profile.id))
                .unwrap()
                .has_password
        );

        let copy = db
            .with(|conn| profiles::duplicate(conn, &profile.id, Some("db-02")))
            .unwrap();
        assert_eq!(copy.name, "db-02");
        assert!(!copy.has_password, "复制配置不复制凭据");

        // 删除父分组：子分组上移、连接保持
        let result = db
            .with_mut(|conn| groups::delete(conn, &group.id, groups::DeleteStrategy::MoveToParent))
            .unwrap();
        assert_eq!(result.deleted_groups, 1);
        assert_eq!(
            db.with(|conn| profiles::get(conn, &profile.id))
                .unwrap()
                .group_id,
            Some(child.id.clone())
        );

        // 级联删除子分组：连接一并删除
        let result = db
            .with_mut(|conn| groups::delete(conn, &child.id, groups::DeleteStrategy::DeleteAll))
            .unwrap();
        assert_eq!(result.deleted_profiles, 2);
        assert!(db.with(|conn| profiles::get(conn, &profile.id)).is_err());
    }

    #[test]
    fn known_hosts_tofu_and_mismatch() {
        use crate::db::repo::known_hosts::{self, VerifyOutcome};
        use crate::error::ErrorCode;

        let db = Db::open_memory(&key()).expect("open");
        let learn = db
            .with(|conn| {
                known_hosts::verify(
                    conn,
                    "example.com",
                    22,
                    "ssh-ed25519",
                    "SHA256:aaa",
                    "ssh-ed25519 AAAA",
                    false,
                )
            })
            .unwrap();
        assert_eq!(learn, VerifyOutcome::Learned);

        let trusted = db
            .with(|conn| {
                known_hosts::verify(
                    conn,
                    "example.com",
                    22,
                    "ssh-ed25519",
                    "SHA256:aaa",
                    "ssh-ed25519 AAAA",
                    true,
                )
            })
            .unwrap();
        assert_eq!(trusted, VerifyOutcome::Trusted);

        let mismatch = db
            .with(|conn| {
                known_hosts::verify(
                    conn,
                    "example.com",
                    22,
                    "ssh-ed25519",
                    "SHA256:bbb",
                    "ssh-ed25519 BBBB",
                    false,
                )
            })
            .unwrap_err();
        assert_eq!(mismatch.code, ErrorCode::SshHostkeyMismatch);

        let strict_unknown = db
            .with(|conn| {
                known_hosts::verify(
                    conn,
                    "new.example.com",
                    22,
                    "ssh-ed25519",
                    "SHA256:ccc",
                    "ssh-ed25519 CCCC",
                    true,
                )
            })
            .unwrap_err();
        assert_eq!(strict_unknown.code, ErrorCode::SshHostkeyUnknown);
    }

    #[test]
    fn settings_partial_update_keeps_defaults() {
        use crate::db::models::{MonitorSettings, SettingsPatch};
        use crate::db::repo::settings;

        let db = Db::open_memory(&key()).expect("open");
        let defaults = db.with(|conn| settings::get(conn)).unwrap();
        assert_eq!(defaults.monitor.interval_ms, 2000);
        assert_eq!(defaults.terminal.font_size, 13);

        let patch = SettingsPatch {
            monitor: Some(MonitorSettings {
                interval_ms: 5000,
                enabled_on_connect: false,
            }),
            ..Default::default()
        };
        let updated = db.with_mut(|conn| settings::update(conn, &patch)).unwrap();
        assert_eq!(updated.monitor.interval_ms, 5000);
        assert_eq!(updated.terminal.font_size, 13, "未提供 section 保持默认");
    }

    #[test]
    fn reopen_file_db_is_idempotent() {
        let path = std::env::temp_dir().join(format!("nestterm-reopen-{}.db", new_id()));
        {
            let db = Db::open_file(&path, &key()).expect("create");
            assert_eq!(db.version(), 1);
        }
        {
            let db = Db::open_file(&path, &key()).expect("reopen");
            assert_eq!(db.version(), 1);
            let groups = db.with(|conn| crate::db::repo::groups::list(conn)).unwrap();
            assert!(groups.is_empty());
        }
        let _ = std::fs::remove_file(&path);
    }

    /// 开发辅助（默认忽略）：向 `NESTTERM_SEED_DB` 指向的加密库写入演示数据，
    /// 便于在无真实服务器时检查界面/树形渲染。
    ///
    /// 用法：
    /// `NESTTERM_SEED_DB=$PWD/.devdata/nestterm.db NESTTERM_DB_KEY_HEX=<64hex> \
    ///  cargo test --lib seed_demo_db -- --ignored --nocapture`
    #[test]
    #[ignore = "dev helper: seeds a demo database"]
    fn seed_demo_db() {
        use crate::db::models::{AuthType, ProfileInput};
        use crate::db::repo::{groups, profiles};

        let path = std::env::var("NESTTERM_SEED_DB").expect("NESTTERM_SEED_DB");
        let key_hex = std::env::var("NESTTERM_DB_KEY_HEX").expect("NESTTERM_DB_KEY_HEX");
        let key = hex::decode(key_hex).expect("hex key");
        let db = Db::open_file(std::path::Path::new(&path), &key).expect("open");

        let prod = db
            .with(|conn| groups::create(conn, None, "生产环境"))
            .unwrap();
        let test = db
            .with(|conn| groups::create(conn, None, "测试环境"))
            .unwrap();

        let demo = |name: &str,
                    host: &str,
                    port: u16,
                    user: &str,
                    group_id: Option<String>,
                    auth: AuthType| ProfileInput {
            name: name.into(),
            group_id,
            host: host.into(),
            port,
            username: user.into(),
            auth_type: auth,
            key_path: if auth == AuthType::Key {
                Some("~/.ssh/id_ed25519".into())
            } else {
                None
            },
            agent_identity: None,
            term: None,
            charset: None,
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: Some(vec!["demo".into()]),
            color: None,
            notes: Some("演示数据（可删除）".into()),
            secret: None,
        };

        for input in [
            demo(
                "prod-db-01",
                "10.0.0.11",
                22,
                "root",
                Some(prod.id.clone()),
                AuthType::Password,
            ),
            demo(
                "prod-web-01",
                "10.0.0.21",
                22,
                "deploy",
                Some(prod.id.clone()),
                AuthType::Key,
            ),
            demo(
                "stage-db",
                "10.0.1.11",
                2222,
                "root",
                Some(test.id.clone()),
                AuthType::Password,
            ),
            demo(
                "jump-host",
                "10.0.1.2",
                22,
                "ops",
                Some(test.id.clone()),
                AuthType::Agent,
            ),
            demo(
                "lab",
                "192.0.2.12",
                22,
                "ubuntu",
                None,
                AuthType::Password,
            ),
        ] {
            db.with(|conn| profiles::create(conn, &input)).unwrap();
        }
        println!("seeded demo db at {path}");
    }

    /// 联调辅助（默认忽略）：向 NESTTERM_SEED_DB 写入一条 key 认证的远程连接。
    #[test]
    #[ignore = "dev helper: seed remote profile"]
    fn seed_remote_profile() {
        use crate::db::models::{AuthType, ProfileInput};
        use crate::db::repo::{groups, profiles};

        let path = std::env::var("NESTTERM_SEED_DB").expect("NESTTERM_SEED_DB");
        let key = hex::decode(std::env::var("NESTTERM_DB_KEY_HEX").expect("key")).expect("hex");
        let db = Db::open_file(std::path::Path::new(&path), &key).expect("open");

        let group = db
            .with(|conn| groups::create(conn, None, "联调环境"))
            .or_else(|_| {
                db.with(|conn| groups::list(conn))
                    .map(|groups| groups.into_iter().find(|g| g.name == "联调环境").unwrap())
            })
            .expect("group");

        let existing = db
            .with(|conn| profiles::list(conn, profiles::GroupFilter::All))
            .expect("list")
            .into_iter()
            .find(|p| p.name == "demo-key");
        if let Some(existing) = existing {
            println!("profile exists: {}", existing.id);
            return;
        }

        let input = ProfileInput {
            name: "demo-key".into(),
            group_id: Some(group.id),
            host: std::env::var("NESTTERM_SEED_HOST").unwrap_or_else(|_| "192.0.2.10".into()),
            port: 22,
            username: std::env::var("NESTTERM_SEED_USER").unwrap_or_else(|_| "demo".into()),
            auth_type: AuthType::Key,
            key_path: Some(
                std::env::var("NESTTERM_SEED_KEY_PATH").expect("NESTTERM_SEED_KEY_PATH"),
            ),
            agent_identity: None,
            term: None,
            charset: None,
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: Some(vec!["联调".into()]),
            color: None,
            notes: Some("自动化联调连接（可删除）".into()),
            secret: None,
        };
        let profile = db
            .with(|conn| profiles::create(conn, &input))
            .expect("create");
        println!("seeded profile: {} {}", profile.id, profile.name);
    }

    /// 联调辅助（默认忽略）：写入一条密码认证的远程连接（不保存凭据）。
    #[test]
    #[ignore = "dev helper: seed password profile"]
    fn seed_password_profile() {
        use crate::db::models::{AuthType, ProfileInput};
        use crate::db::repo::{groups, profiles};

        let path = std::env::var("NESTTERM_SEED_DB").expect("NESTTERM_SEED_DB");
        let key = hex::decode(std::env::var("NESTTERM_DB_KEY_HEX").expect("key")).expect("hex");
        let db = Db::open_file(std::path::Path::new(&path), &key).expect("open");

        let group = db
            .with(|conn| groups::create(conn, None, "联调环境"))
            .or_else(|_| {
                db.with(|conn| groups::list(conn))
                    .map(|groups| groups.into_iter().find(|g| g.name == "联调环境").unwrap())
            })
            .expect("group");

        let existing = db
            .with(|conn| profiles::list(conn, profiles::GroupFilter::All))
            .expect("list")
            .into_iter()
            .find(|p| p.name == "demo-pass");
        if let Some(existing) = existing {
            println!("profile exists: {}", existing.id);
            return;
        }

        let input = ProfileInput {
            name: "demo-pass".into(),
            group_id: Some(group.id),
            host: std::env::var("NESTTERM_SEED_HOST").unwrap_or_else(|_| "192.0.2.10".into()),
            port: 22,
            username: std::env::var("NESTTERM_SEED_USER").unwrap_or_else(|_| "demo".into()),
            auth_type: AuthType::Password,
            key_path: None,
            agent_identity: None,
            term: None,
            charset: None,
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: Some(vec!["联调".into()]),
            color: None,
            notes: Some("密码认证联调（不保存密码）".into()),
            secret: None,
        };
        let profile = db
            .with(|conn| profiles::create(conn, &input))
            .expect("create");
        println!("seeded profile: {} {}", profile.id, profile.name);
    }
}
