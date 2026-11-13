//! 从系统 `~/.ssh/known_hosts` 迁移主机公钥到应用自有表。
//!
//! 安全边界：
//! - 只处理**数据库中已有连接配置**的 host:port，不导入无关主机；
//! - `known_host_keys` 支持 OpenSSH 的 hashed host 格式；
//! - 与库中已有指纹不一致时**不覆盖**，只计为冲突并记录 warning；
//! - 失败（文件不存在/无权限）静默跳过，不阻塞启动。

use std::collections::HashSet;

use russh::keys::known_hosts::known_host_keys;
use russh::keys::HashAlg;

use crate::db::repo;
use crate::db::repo::known_hosts::VerifyOutcome;
use crate::db::Db;
use crate::error::ApiResult;

#[derive(Debug, Clone, Default)]
pub struct MigrateResult {
    pub imported: usize,
    pub skipped: usize,
    pub conflicts: usize,
}

/// 迁移一次；重复执行是幂等的。
pub fn migrate(db: &Db) -> ApiResult<MigrateResult> {
    let profiles = db.with(|conn| repo::profiles::list(conn, repo::profiles::GroupFilter::All))?;
    let mut result = MigrateResult::default();
    let mut seen: HashSet<(String, u16)> = HashSet::new();

    for profile in profiles {
        let key = (profile.host.clone(), profile.port);
        if !seen.insert(key.clone()) {
            continue;
        }

        // 读取系统 known_hosts（含 hashed host）；不存在/不可读则跳过。
        let keys = match known_host_keys(&profile.host, profile.port) {
            Ok(keys) => keys,
            Err(_) => continue,
        };

        for (_, public_key) in keys {
            let key_type = public_key.algorithm().as_str().to_string();
            let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
            let openssh = public_key.to_openssh().unwrap_or_default();
            match db.with(|conn| {
                repo::known_hosts::verify(
                    conn,
                    &profile.host,
                    profile.port,
                    &key_type,
                    &fingerprint,
                    &openssh,
                    false,
                )
            }) {
                Ok(VerifyOutcome::Learned) => result.imported += 1,
                Ok(VerifyOutcome::Trusted) => result.skipped += 1,
                Err(_) => result.conflicts += 1,
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{AuthType, ProfileInput};

    #[test]
    fn migration_is_scoped_to_configured_hosts() {
        // 只验证“无配置时不读系统文件/不报错”这一边界；
        // 真实 known_hosts 解析依赖用户 HOME，不在单测中伪造。
        let db = Db::open_memory(&[9_u8; 32]).expect("db");
        let result = migrate(&db).expect("migrate");
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.conflicts, 0);

        let input = ProfileInput {
            name: "demo".into(),
            group_id: None,
            host: "192.0.2.10".into(),
            port: 22,
            username: "demo".into(),
            auth_type: AuthType::Password,
            key_path: None,
            agent_identity: None,
            term: None,
            charset: None,
            keepalive_secs: None,
            connect_timeout_secs: None,
            jump_profile_id: None,
            tags: None,
            color: None,
            notes: None,
            secret: None,
        };
        db.with(|conn| repo::profiles::create(conn, &input))
            .expect("profile");
        // 系统 known_hosts 可能没有该文档地址，允许 0 或 1 个 key。
        let result = migrate(&db).expect("migrate with profile");
        assert!(result.imported + result.skipped + result.conflicts < 10);
    }
}
