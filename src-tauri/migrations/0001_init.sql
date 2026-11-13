-- NestTerm schema v1：与 docs/03-database-schema.md §6 保持逐字一致
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
