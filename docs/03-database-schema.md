# NestTerm 数据库与凭据存储设计

> 文档版本：v0.1（2026-10-26） · 对应代码：0.1.0（A 期，尚未接库）
> 同组文档：[产品范围](00-product-scope.md) · [界面布局](01-ui-layout.md) · [IPC 契约](02-ipc-contract.md)

目标：连接配置可长期演进、可备份、可迁移；**任何敏感信息不以明文落盘**；单人迭代下不引入运维复杂度。

---

## 1. 存储分层

| 层 | 存什么 | 技术 | 是否加密 |
| --- | --- | --- | --- |
| 结构化配置 | 分组、连接、转发规则、known_hosts、设置 | SQLite（SQLCipher） | 是（整库） |
| 凭据 | 密码、私钥 passphrase、DB 主密钥 | 系统钥匙串（macOS Keychain / Windows Credential Manager / Linux Secret Service） | 由系统保护 |
| 文件 | 终端日志、回放、导出文件、临时下载 | 应用数据目录 | 否（日志本身可能含敏感输出，需提示） |
| 内存 | 运行中的 SSH 会话、SFTP 句柄、监控采样 | Rust 进程内 | 否（进程退出即释放） |
| 前端 | 当前表单输入、滚动位置、面板尺寸 | Vue 内存 + 少量 localStorage（仅 UI 偏好） | 否（**禁止**存任何凭据） |

**关键决策：密码不进数据库。** 数据库加密保护的是主机名、用户名、备注等元数据；密码/口令只进系统钥匙串。这样即使 DB 密钥泄露，凭据仍需系统级授权才能取用。

## 2. 路径与权限

| 平台 | 数据目录 | DB 文件 |
| --- | --- | --- |
| macOS | `~/Library/Application Support/com.nestterm.desktop/` | `nestterm.db` |
| Windows | `%APPDATA%\com.nestterm.desktop\` | `nestterm.db` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/com.nestterm.desktop/` | `nestterm.db` |

子目录：

```
<data>/
├─ nestterm.db          # SQLCipher 加密库（WAL 模式会有 -wal/-shm）
├─ nestterm.db.bak.1    # 迁移前自动备份（滚动保留 2 份：.bak.1 / .bak.2）
├─ logs/                # 终端日志（P1）
├─ backups/             # 手动导出/VACUUM INTO 备份
└─ tmp/                 # 下载临时文件（.part），启动时清理 > 7 天
```

- Unix 权限：目录 `0700`，DB/备份文件 `0600`（Windows 依赖用户目录 ACL）。
- 路径统一由 Tauri `app_data_dir()` 解析，**不硬编码**；测试可用 `NESTTERM_DATA_DIR` 覆盖（仅 debug 构建）。

## 3. 加密（SQLCipher）

### 3.1 选型

`rusqlite` + `bundled-sqlcipher-vendored-openssl`：

- 整库 AES-256 加密，页级透明；应用层 SQL 与普通 SQLite 一致。
- vendored OpenSSL 避免依赖系统 `libcrypto`（macOS 为 LibreSSL，不兼容），代价是首次编译较慢（一次性）。
- 备选（不采用）：字段级 AES-GCM。需要自己维护每列加解密，且 `LIKE`/排序会失效；仅在 SQLCipher 编译受阻时作为降级方案。

### 3.2 密钥生命周期

1. **首次启动**：生成 32 字节 CSPRNG 随机密钥 → base64 → 写入钥匙串（服务 `com.nestterm.desktop`，账户 `db-key-v1`）。
2. **打开数据库**：

```rust
let key = keyring::Entry::new("com.nestterm.desktop", "db-key-v1")?.get_password()?;
let bytes = BASE64.decode(key)?;                 // 且校验长度 = 32
conn.pragma_update(None, "key", format!("x'{}'", hex::encode(bytes)))?;
// 触发一次读取以验证密钥；失败即密钥错误/文件损坏
conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
```

3. **换机/恢复**：钥匙串不随备份迁移 → 导出时提供「带口令的加密导出」，在新机导入时重建钥匙串项。
4. **轮换（P1）**：`PRAGMA rekey` 前先 `VACUUM INTO` 备份；成功后更新钥匙串。**发版流程不自动 rekey**，避免失败导致库不可读。
5. **密钥丢失**：DB 不可恢复（这是加密的代价）。UI 须在「关于/安全」中明确提示，并提供导出备份入口。

### 3.3 钥匙串不可用（Linux 无 Secret Service / 无桌面会话）

- 启动检测 `keyring::Entry::get_password()` 返回 `NoStorageAccess` 等错误。
- 走**主口令模式**：弹窗让用户设置主口令（≥ 8 字符）→ Argon2id（m=64MB, t=3, p=1）派生 32 字节密钥 → 用该密钥 AES-256-GCM 加密 DB 密钥，存 `<data>/db.key.enc`。
- 每次启动输入主口令解锁；不写入内存以外的地方。
- 该模式需在 UI 明示「安全性依赖主口令强度」；`SECRET` 类操作（保存密码）在此模式下**禁用**，改为每次连接输入，避免把明文交给无系统保护的存储。

### 3.4 其他 SQLCipher 设置

```sql
PRAGMA cipher_memory_security = ON;   -- 降低内存残留；采样期间性能影响可接受
PRAGMA foreign_keys = ON;             -- 每个连接都要设，非持久化
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
```

- 备份/导出的文件保持加密（`VACUUM INTO` 产物同为 SQLCipher 库；导入方需同一密钥）。
- 不使用 `PRAGMA key` 的明文密码形式；只用 `x'hex'` 原始密钥，避免 KDF 差异。

## 4. 凭据（系统钥匙串）

| 项 | 值 |
| --- | --- |
| 服务名 | `com.nestterm.desktop` |
| DB 密钥账户 | `db-key-v1` |
| 密码账户 | `profile/<profileId>/password` |
| passphrase 账户 | `profile/<profileId>/passphrase` |
| 可访问性（macOS） | `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`（不随 iCloud 同步） |

规则：

1. **只写不读回前端**：IPC 层永不返回 secret；`ConnProfile` 只暴露 `hasPassword/hasPassphrase`。
2. **写入时机**：`profile_create/update` 携带 `secret` 时写钥匙串；写失败则整体失败并回滚 DB 变更。
3. **删除顺序**：先删钥匙串项，再删 DB 行；钥匙串删除失败则中止（返回 `KEYCHAIN_ERROR`），避免留下“看不见但仍在”的凭据。
4. **一致性补偿**：若钥匙串已删、DB 删除失败，把 `has_password/has_passphrase` 置 0 并返回部分失败，提示用户重试。
5. **内存**：密码读取后使用 `zeroize` 清零的 `Vec<u8>`；不进入 `Debug` 输出（凭据类型不派生 `Debug`）。
6. **连接失败不缓存**：认证失败的密码不保留在内存；用户重试需重新读取钥匙串或重输。

依赖特性（`keyring` v3）：macOS `apple-native`、Windows `windows-native`、Linux `sync-secret-service`（DBus Secret Service）。Linux 需按平台声明：

```toml
[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "3", features = ["sync-secret-service"] }
```

无桌面会话（headless）时按 §3.3 降级为主口令模式。

## 5. 连接与并发模型

- 单进程、单写连接：`Mutex<Connection>`（`std::sync::Mutex`，临界区 < 5ms，不跨 `.await`）。
- 重活（导入 1000 条、导出、`VACUUM`）走 `tokio::task::spawn_blocking`，持有同一个连接，UI 期间显示进度。
- 读操作也走同一连接（SQLite 读很快），避免 WAL 下多连接带来的可见性心智负担。
- 事务边界：批量导入/排序重排/分组删除必须单事务；单条 CRUD 自动提交。
- 时间戳由 Rust 统一生成 `now_ms()`；SQL 默认值不写 `CURRENT_TIMESTAMP`（避免秒级精度与时区问题）。
- `sort_order` 采用**稀疏整数**（步长 1000）插入；重排时在事务内对同组全量重编号（组内条目 ≤ 1000 时成本可忽略）。

## 6. DDL（schema v1 · B 期落地）

> 落地位置：`src-tauri/migrations/0001_init.sql`。执行后 `PRAGMA user_version = 1`。
> 约定：`id` 均为 UUIDv7 文本；时间均为 Unix 毫秒；布尔用 `0/1`。

```sql
-- ============ 分组 ============
CREATE TABLE groups (
  id          TEXT PRIMARY KEY NOT NULL,
  parent_id   TEXT REFERENCES groups(id) ON DELETE CASCADE,
  name        TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 64),
  icon        TEXT,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX ux_groups_sibling_name
  ON groups (COALESCE(parent_id, ''), name);
CREATE INDEX ix_groups_parent ON groups (parent_id, sort_order);

-- ============ 连接配置 ============
CREATE TABLE profiles (
  id                   TEXT PRIMARY KEY NOT NULL,
  group_id             TEXT REFERENCES groups(id) ON DELETE SET NULL,
  name                 TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 64),
  host                 TEXT NOT NULL CHECK (length(trim(host)) BETWEEN 1 AND 255),
  port                 INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
  username             TEXT NOT NULL CHECK (length(trim(username)) BETWEEN 1 AND 64),
  auth_type            TEXT NOT NULL CHECK (auth_type IN ('password','key','agent')),
  key_path             TEXT,
  key_fingerprint      TEXT,
  agent_identity       TEXT,
  has_password         INTEGER NOT NULL DEFAULT 0 CHECK (has_password IN (0,1)),
  has_passphrase       INTEGER NOT NULL DEFAULT 0 CHECK (has_passphrase IN (0,1)),
  term                 TEXT NOT NULL DEFAULT 'xterm-256color',
  charset              TEXT NOT NULL DEFAULT 'utf-8',
  keepalive_secs       INTEGER NOT NULL DEFAULT 30 CHECK (keepalive_secs BETWEEN 0 AND 600),
  connect_timeout_secs INTEGER NOT NULL DEFAULT 20 CHECK (connect_timeout_secs BETWEEN 5 AND 120),
  jump_profile_id      TEXT REFERENCES profiles(id) ON DELETE SET NULL,
  tags                 TEXT NOT NULL DEFAULT '[]'
                       CHECK (json_valid(tags) AND json_type(tags) = 'array'),
  color                TEXT,
  notes                TEXT,
  sort_order           INTEGER NOT NULL DEFAULT 0,
  last_connected_at    INTEGER,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  CHECK (auth_type <> 'key' OR (key_path IS NOT NULL AND length(key_path) > 0)),
  CHECK (jump_profile_id IS NULL OR jump_profile_id <> id)
);
CREATE INDEX ix_profiles_group ON profiles (group_id, sort_order);
CREATE INDEX ix_profiles_host ON profiles (host);
CREATE INDEX ix_profiles_last ON profiles (last_connected_at DESC);
CREATE INDEX ix_profiles_name ON profiles (name);

-- ============ 主机公钥（替代 ~/.ssh/known_hosts） ============
CREATE TABLE known_hosts (
  id            TEXT PRIMARY KEY NOT NULL,
  host          TEXT NOT NULL,
  port          INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
  key_type      TEXT NOT NULL,
  fingerprint   TEXT NOT NULL,
  public_key    TEXT NOT NULL,          -- OpenSSH 单行格式
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  UNIQUE (host, port, key_type)
);

-- ============ 端口转发规则 ============
CREATE TABLE forward_rules (
  id          TEXT PRIMARY KEY NOT NULL,
  profile_id  TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('local','remote','dynamic')),
  bind_host   TEXT NOT NULL DEFAULT '127.0.0.1',
  bind_port   INTEGER NOT NULL CHECK (bind_port BETWEEN 1 AND 65535),
  target_host TEXT,
  target_port INTEGER CHECK (target_port BETWEEN 1 AND 65535),
  auto_start  INTEGER NOT NULL DEFAULT 0 CHECK (auto_start IN (0,1)),
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  CHECK (
    (kind = 'dynamic' AND target_host IS NULL AND target_port IS NULL)
    OR (kind IN ('local','remote') AND target_host IS NOT NULL AND target_port IS NOT NULL)
  )
);
CREATE INDEX ix_forward_profile ON forward_rules (profile_id);
-- 本地监听端口冲突（-L 与 -D 共用本地网络栈；-R 绑定在远端，不参与）
CREATE UNIQUE INDEX ux_forward_local_bind
  ON forward_rules (profile_id, bind_host, bind_port)
  WHERE kind IN ('local','dynamic');

-- ============ 设置（KV，值为 JSON 文本） ============
CREATE TABLE settings (
  key        TEXT PRIMARY KEY NOT NULL,
  value      TEXT NOT NULL CHECK (json_valid(value)),
  updated_at INTEGER NOT NULL
);

PRAGMA user_version = 1;
```

### 6.1 表说明与设计理由

**groups**

- `parent_id` 自引用，`ON DELETE CASCADE` 仅作兜底；业务删除由应用按 `childrenStrategy` 显式处理（移入父级或级联删）。
- 同级唯一名用**表达式索引** `COALESCE(parent_id,'')`，解决 SQLite `NULL` 不参与唯一约束的问题。
- 层级建议 ≤ 3；应用层校验深度，避免 UI 无限缩进。

**profiles**

- `group_id ON DELETE SET NULL`：删除分组时连接进入「未分组」，不丢数据。
- `auth_type` 三态；`key_path` 仅 key 类型必填（CHECK 保证）。
- `has_password/has_passphrase` 是钥匙串的**冗余镜像**，用于 UI 显示“已保存”而无需访问钥匙串；写入/删除 secret 时同事务更新。
- `tags` 用 JSON 数组（P1 标签筛选），`json_valid` + `json_type` 双保险；不建关联表，保持 P0 简单。
- `jump_profile_id` 为 P1 跳板机预留：自引用 + 禁止指向自身；删除跳板配置时置空（连接变直连，UI 提示）。
- 不设 `UNIQUE(group_id, name)`：允许同名连接（不同环境同名很常见），避免导入时被迫改名。

**known_hosts**

- 应用自有表，不读写用户 `~/.ssh/known_hosts`（A 期为过渡实现）。
- 唯一键 `(host, port, key_type)` 支持同一主机多算法并存（如 RSA 与 ed25519）。
- `public_key` 存 OpenSSH 单行，便于导出与人工核对；`fingerprint` 为 SHA256 展示值。
- 首次启动提供一次性「从 ~/.ssh/known_hosts 导入」选项（只读，不改动系统文件）；导入解析失败逐条 warning。

**forward_rules**

- 与 profile 绑定（`ON DELETE CASCADE`），随连接配置删除。
- `kind=dynamic` 不允许 target；`local/remote` 必须给 target（CHECK 保证）。
- 本地端口唯一索引仅覆盖 `local/dynamic`；`remote` 的绑定发生在服务器侧，端口可与本地重复。
- 运行时状态（running/bytes）**不落库**，只在内存与会话生命周期内存在；应用重启后按 `auto_start` 重建。

**settings**

- KV + JSON 值，新增设置项无需迁移表结构；读取时与代码内默认值深合并。
- 字符串值也必须 JSON 编码（如 `"dark"` 而非 `dark`），保证 `json_valid`。

### 6.2 未建表但已在 IPC 出现的对象

| 对象 | 存储位置 | 原因 |
| --- | --- | --- |
| Session / Transfer / Forward 实例 | 内存 | 生命周期 = 进程；持久化无意义且易产生僵尸状态 |
| 监控采样 | 内存（60 点环形缓冲） | 只需近 2 分钟曲线；落盘徒增 IO |
| 最近打开列表 | `profiles.last_connected_at` | 无需单独表 |
| 导入批次回滚 | 不落盘 | P0 仅在向导内提供“撤销本次导入”（内存记录 id 列表） |

## 7. 迁移策略

1. 启动顺序：解析路径 → 取钥匙串密钥 → 打开 DB → 校验 `user_version` → 需要时迁移 → 业务可用。
2. 版本规则：
   - `user_version == TARGET`：正常启动。
   - `user_version < TARGET`：**先备份** `nestterm.db.bak.<n>`（保留 2 份）→ 逐个执行 `migrations/000N_*.sql`（单事务）→ 更新 `user_version`。
   - `user_version > TARGET`：拒绝启动，返回 `DB_MIGRATION`（提示“数据库由更新版本创建”，防止旧客户端写坏新库）。
3. 迁移文件**只增不改**：已发布的 `0001_init.sql` 永不修改（改表必须新增 `0002_*.sql`）。
4. 每个迁移必须有对应测试：用 fixture DB（v0 空库 + v1 样本库）跑升级，断言表结构、索引、关键数据完好。
5. 迁移失败：回滚事务、保留备份、弹窗给出备份路径；不允许“部分迁移后继续运行”。

**v2（P1，先设计不落地）**

> 落地时新建 `migrations/0002_p1_tables.sql`，并在文件末尾追加 `PRAGMA user_version = 2;`。

```sql
-- 命令片段库
CREATE TABLE snippet_groups (
  id TEXT PRIMARY KEY NOT NULL,
  name TEXT NOT NULL UNIQUE,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE snippets (
  id TEXT PRIMARY KEY NOT NULL,
  group_id TEXT REFERENCES snippet_groups(id) ON DELETE SET NULL,
  name TEXT NOT NULL,
  command TEXT NOT NULL,
  description TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- 终端日志索引（文件本体在 logs/ 下）
CREATE TABLE terminal_logs (
  id TEXT PRIMARY KEY NOT NULL,
  profile_id TEXT REFERENCES profiles(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  file_name TEXT NOT NULL,              -- 相对于 <data>/logs/
  format TEXT NOT NULL DEFAULT 'raw' CHECK (format IN ('raw','asciinema')),
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  size_bytes INTEGER NOT NULL DEFAULT 0,
  CHECK (ended_at IS NULL OR ended_at >= started_at)
);
```

## 8. 备份、导出与导入

### 8.1 本地备份

- 迁移前自动备份（见 §7）。
- 手动备份：`VACUUM INTO '<data>/backups/nestterm-<ts>.db'`（SQLCipher 库，需同一密钥打开）。
- 保留策略：自动备份仅保留 2 份；手动备份不自动删除。

### 8.2 用户导出（跨机迁移）

默认导出**不含凭据**。勾选“包含凭据”时：

```json
{
  "format": "nestterm-export",
  "version": 1,
  "exportedAt": 1761475200000,
  "groups": [ ... ],
  "profiles": [ ... ],
  "forwardRules": [ ... ],
  "secrets": {
    "<profileId>": { "password": "<base64 AES-256-GCM>", "passphrase": null }
  },
  "kdf": { "algo": "argon2id", "m": 65536, "t": 3, "p": 1, "salt": "<base64>" }
}
```

- 密钥由用户口令经 Argon2id 派生；每个 secret 独立随机 nonce，AAD = `profileId + "\x00" + kind`。
- 文件不含 DB 密钥；导入到新机时重建钥匙串项。
- 未勾选凭据时 `secrets` 字段省略，导入后 `hasPassword=false`，首次连接提示输入。

### 8.3 FinalShell 导入

- 只读解析，不执行任何内容；字段映射表见 [import-mapping.md](import-mapping.md)（B 期用真实样本确定）。
- 导入写库在一个事务内完成；单条失败跳过并累计 warning。
- 若源数据含明文密码：默认**不导入**（提示「检测到 N 条明文密码，建议导入后重新输入」）；用户显式勾选才写入钥匙串，且立即提示风险。

## 9. 数据保留与清理

| 数据 | 策略 |
| --- | --- |
| 自动备份 | 保留最近 2 份，超出删除最旧 |
| `tmp/` 下载临时文件 | 启动时清理 > 7 天；下载成功即改名移除 |
| 终端日志（P1） | 默认关闭；开启后按 `logs.retentionDays`（默认 30）清理；UI 提供“全部删除” |
| 已删除连接的钥匙串项 | 删除连接时同步删除（见 §4.3） |
| WAL 文件 | 正常关闭时 checkpoint；异常退出后由 SQLite 自动恢复 |

## 10. 性能与容量预期

| 指标 | 目标 |
| --- | --- |
| 500 条连接 + 50 分组，全量树查询 | < 5ms |
| 单条 CRUD | < 2ms |
| 1000 条导入（DB 写入，不含解析与钥匙串） | < 300ms |
| 典型 DB 体积 | < 10MB（不含日志/备份） |
| 冷启动打开 DB（含解密） | < 150ms |
| 迁移 v1（空库） | < 50ms |

- 列表搜索用 `LIKE '%kw%'`（500 条规模足够）；超过 5000 条再考虑 FTS5（预留，不提前优化）。
- `profiles` 上避免 `SELECT *` 到前端时带大字段：`notes` 可能很长，列表查询只取摘要，编辑时单独 `profile_get`。

## 11. B 期落地清单

1. `src-tauri/migrations/0001_init.sql`（内容 = §6，逐字落地）。
2. `src-tauri/src/db/mod.rs`：打开/迁移/便捷查询；`dbVersion` 暴露给 `app_info`。
3. `src-tauri/src/db/keychain.rs`：`keyring` 封装（服务名/账户命名、错误码映射、zeroize）。
4. `src-tauri/src/db/repo/{profiles,groups,known_hosts,forward_rules,settings}.rs`：SQL 与领域类型互转。
5. 导入/导出模块：`src-tauri/src/portable/`（FinalShell 映射、JSON 导出、Argon2id + AES-GCM）。
6. 契约测试：`cargo test` 覆盖迁移（v0→v1）、约束（CHECK 生效）、钥匙串缺失降级。
7. 启动时一次性 known_hosts 迁移（`~/.ssh/known_hosts` → 表，只读源文件，失败不阻塞启动）。

## 12. 待确认（B 期开工前）

1. Linux 目标发行版是否都有 Secret Service（决定主口令降级路径的优先级）。
2. 是否需要「便携模式」（数据目录跟随可执行文件，用于 U 盘）—— 会改变路径解析与安全模型。
3. FinalShell 导出样本的字段结构（决定 `import-mapping.md`；请提供一份脱敏样本）。

## 13. B1 实施状态（2026-10-26）

| 项 | 状态 |
| --- | --- |
| `migrations/0001_init.sql` | ✅ 已落地，内容与 §6 逐字一致 |
| SQLCipher | ✅ `rusqlite 0.41 + bundled-sqlcipher-vendored-openssl`；DB 文件头已非明文 SQLite |
| 迁移与备份 | ✅ `user_version` 递增；文件库迁移前复制 `.bak.1/.bak.2`；降级拒绝打开 |
| 连接参数 | ✅ WAL / `foreign_keys=ON` / `busy_timeout=5000` / `synchronous=NORMAL` |
| 钥匙串 | ✅ `keyring 3.6.3`（`apple-native`）；账户命名按 §4；`MemoryStore` 供单测/调试 |
| 仓储 | ✅ groups / profiles / known_hosts / settings 全量 CRUD 与约束 |
| 单测 | ✅ 7 个：迁移、约束、错误密钥、分组/连接 CRUD、TOFU、设置合并、重开幂等 |
| 调试开关 | `NESTTERM_DATA_DIR`、`NESTTERM_DB_KEY_HEX`、`NESTTERM_MEMORY_SECRETS=1`（仅 debug 构建） |
| 未实现 | 主口令降级（headless Linux）、导出/导入、日志保留、`rekey`、旧 known_hosts 迁移 |

> B1 未在真实 Keychain 授权路径上做自动化验收（沙箱拒绝 UI 授权），以 `MemoryStore` + 加密文件库覆盖逻辑；
> 发布前需在普通用户会话中手工验证一次钥匙串授权与重启读取。

### B2a 补充（2026-10-26）

- **无 schema 变更**：SFTP 复用同一条 SSH 连接的独立 channel，不新增持久化表；传输历史仍只在内存（B2b 再评估是否落盘）。
- 新增单测：OSC 7 cwd 解析 5 个、SFTP 路径/属性 4 个；`cargo test --lib` 共 **16 passed**（DB 7 + cwd 5 + sftp 4）。
- cwd 解析在 Rust 内完成，不消费终端字节；前端仅按 `ssh:cwd` 驱动目录跟随。

### B2a 实机联调（2026-10-26）

- 测试机：Ubuntu 24.04.4 LTS（kernel 6.17），应用内密码认证与 ed25519 私钥认证均连接成功。
- SFTP 命令层自检（真实 IPC）：mkdir → write → read → chmod 600 → stat → rename → list → 递归删除，全部通过；
  报告写在服务器 `~/nestterm-selftest.json`，`"ok": true`。
- OSC 7 → `ssh:cwd` → 右侧文件面板跟随到 `/home/demo/nestterm-live`。
- 未覆盖：SFTP 上传/下载、窗口 resize 动态变化、vim/tmux（测试机未安装）。

### B2b-1 文件传输（2026-10-26）

- 无 schema 变更；传输任务仅内存态（`transferId -> AtomicBool` 取消标记），不落库。
- 后端：单文件上传/下载、64KB 分块、100ms 进度节流、取消清理临时文件、`ask/overwrite/skip/rename` 冲突策略。
- 实机：8MB 上传/覆盖/下载双端 SHA-256 一致；64MB 中途取消无 `.part`/无最终文件；报告 `~/nestterm-transfer-selftest.json`、`~/nestterm-cancel-selftest.json`。
- 修复：OpenSSH SFTP `rename` 不覆盖目标 → 覆盖前临时备份旧文件，失败回滚。
- 单测总数：**18 passed / 3 ignored**。

### B2b-2a 目录传输（2026-10-26）

- 目录在上传/下载前先展开为文件计划；空目录也会创建。
- 单次请求文件数上限 2000；symlink 跳过（避免循环）。
- 冲突策略作用于顶层目标与已存在的嵌套文件；覆盖目录为 merge 语义。
- 实机：嵌套目录 + 空目录 + 隐藏文件 + 2MB 二进制，4 个文件双端 SHA-256 全部一致。
- 取消仍为单文件粒度；目录整体取消/断点续传属后续。

### B2b-2b FinalShell 导入（2026-10-26）

- 无 schema 变更；导入复用 `groups/profiles` 仓储。
- 实测 macOS FinalShell 4.6.5：`~/Library/FinalShell/conn/*_connect_config.json` 为明文 JSON。
- 3 条真实连接导入成功（名称/主机/端口/用户名/备注）；密码字段（32 字符 base64）未解密、未入库。
- 自动创建/复用分组「FinalShell 导入」；按 `(host,port,username)` 去重，二次导入全部跳过。
- 字段映射见 [import-mapping.md](import-mapping.md)。

### B2b-2c JSON 导出（2026-10-26）

- 无 schema 变更；导出只读 DB，不触碰钥匙串。
- 输出 `format=nestterm-export, version=1`，分组用路径表达，连接不含 password/passphrase/私钥。
- 实机：10 个连接 / 4 个分组 / 5.2KB；`credentialsExcluded=true`；字段检查无凭据。
- NestTerm JSON 导入已实现（B2b-2d）：事务内重建分组路径、按 host/port/user 去重；全新空库导入 10 连接/4 分组实测通过。

### B2b-2e 迁移与下载目录（2026-10-26）

- known_hosts 迁移：启动时读取 `~/.ssh/known_hosts`，仅针对数据库中已有 profile 的 host:port；
  使用 `russh::keys::known_hosts::known_host_keys`（支持 hashed host），走 `known_hosts::verify` 写入；
  指纹冲突不覆盖；失败不阻塞启动；幂等。
- 下载目录选择：新增 `tauri-plugin-dialog`，SFTP 下载前由用户选择目录；取消则中止。
  后端 `sftp_download.localDir` 已在实机使用自定义目录验证。

### B2b-2f cd 启发式（2026-10-26）

- 无 schema 变更；纯运行时输入解析。
- 解析简单 `cd`（绝对/相对/`~`/引号/`..`/`cd -`），含 shell 复合语法的行忽略。
- 候选路径由前端 `sftp_realpath` 校验，失败静默忽略；OSC 7 事件优先级更高。
- 实机（未配置 PROMPT_COMMAND/OSC 7）：`cd /home/demo/nestterm-live` 后文件面板正确跟随。
- 单测：5 个输入解析用例；总计 **30 passed / 4 ignored**。

### B3a 本地转发 / SOCKS5（2026-10-26）

- 复用 v1 `forward_rules` 表，无 schema 变更。
- 后端：`forward_start/stop` 管理监听任务；每条本地 TCP 连接开一个 `direct-tcpip` channel。
- 实机：`-L 127.0.0.1:2222 → 服务器 127.0.0.1:22`、`-D 127.0.0.1:1080`（SOCKS5 → 192.0.2.10:22）均成功读取 SSH banner。
- 会话断开时停止该会话的监听；应用重启不自动恢复。
- 远端 `-R` 与 SOCKS5 认证尚未实现。

### B3b 系统监控（2026-10-26）

- 无 schema 变更；监控数据不落库。
- 每次采样开一个 exec channel，运行只读 `/proc` + `df` 脚本；CPU 用 200ms 两次快照差值。
- 面板可见时前端每 2s 调 `monitor_sample`，关闭即停止。
- 实机 Ubuntu：CPU/内存 16.6%/负载/磁盘 6.0%/累计网络正常显示。
- 仅支持 Linux；无历史/告警；网络为累计字节。

### B3c 远端转发 -R（2026-10-26）

- 复用 v1 `forward_rules`，无 schema 变更。
- `forward_start` 对 remote 调 `tcpip_forward(address, port)`；客户端 Handler 处理 `forwarded-tcpip`，
  按端口查目标映射并连本地目标，桥接到 SSH channel。
- 停止时 `cancel_tcpip_forward`；会话断开时停止该会话转发。
- 实机：服务器 `127.0.0.1:9000` → Mac `127.0.0.1:18080` HTTP，`remote-forward-ok`；停止后服务器端口消失。
- 边界：依赖服务器 `AllowTcpForwarding`；目标必须是运行 NestTerm 的本机可达地址；SOCKS5 无认证。

### B3c 自动重连（2026-10-26）

- 无 schema 变更；后端 `ssh/reconnect.rs` 保存连接计划（凭据 Zeroizing），断线后复用同一 `sessionId` 重建。
- 仅对已保存密码/私钥的配置生效；最多 3 次，700ms×次数退避；用户主动断开不重连。
- 重连成功后重启 pump、恢复转发规则；PTY 尺寸取最近一次 `ssh_resize`。
- 实测：`sleep 3; exit` 后同 sessionId 重建；2222/1080/9000 三个转发全部恢复。

### UX 加固（2026-10-27）

- 会话恢复：`sessions_save/saved` 将打开的 profile id 存 settings KV（`open_profiles`），启动时按 `restoreSessions` 恢复；强杀重启实测通过。
- 拖拽下载：`sftp_drag_out` + `tauri-plugin-dragout`，drop 时才下载；文件/目录均支持（目录递归）。
- 设置：终端/SFTP/监控/自动重连/关闭确认/host key 策略持久化并应用。
- CSP：release/dev 均设置；Element Plus 按需引入（JS 846KB / CSS 159KB）。
- PTY 尺寸：`ssh_resize` 更新重连上下文，重连使用最近尺寸。
- CI：`.github/workflows/ci.yml`（未签名）、`release.yml`（签名/公证）；本地 `scripts/package-macos.sh`。

### SFTP 目录列表容错（2026-10-27）

- `sftp_list` 的 `realpath` 只做规范化；失败时回退请求路径继续 `read_dir`。
- `read_dir` 失败重试一次（120ms）；仍失败时回退到同一 SSH 会话的 `find -H -printf`（跟随顶层 symlink，NUL 分隔），避免部分服务器 SFTP OPENDIR/READDIR 异常导致整目录不可用。
- 文件操作仍走 SFTP；fallback 仅用于列目录；`cd` 候选先 `test -d` 校验，且解析器只在列表成功后提交 current。

### cwd 跟随最终方案（2026-10-27）

- 权威来源：shell 提示符（`user@host:~/path$`），在 xterm 输出后解析；`~` 用 SFTP home 展开。
- 启发式仅在提示符无法识别时兜底；识别到提示符后忽略启发式事件。
- 列表请求带递增导航序号；首屏 home 等过期结果不会覆盖新目录。
- current 只在被采用的最新列表后提交；SFTP 列目录失败会重置会话/readlink/find 兜底。

### OSC 7 / 输入启发式删除（2026-10-27）

- `ssh/cwd.rs`、`ssh:cwd` 事件、后端 cwd 槽与输入启发式联动代码已删除。
- 当前 cwd 只由前端 shell 提示符解析；导航序号与列表容错保留。
- `cargo test --lib` 更新为 26 passed / 4 ignored（删除 OSC 7 的 5 个单测）。
