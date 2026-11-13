# NestTerm IPC 通信契约

> 文档版本：v0.1（2026-10-26） · 对应代码：0.1.0（A 期，部分实现）
> 同组文档：[产品范围](00-product-scope.md) · [界面布局](01-ui-layout.md) · [数据库设计](03-database-schema.md)

本文是**前后端唯一契约**。B 期实现以本文为准；与 A 期已落地的差异集中在 [§9](#9-a--b-契约差异必须按序处理)。

---

## 1. 通用约定

| 项 | 约定 |
| --- | --- |
| 命名 | Rust 命令 `snake_case`；TS 参数与结构体字段 `camelCase`（Rust 侧 `#[serde(rename_all = "camelCase")]`） |
| 调用 | `invoke('command_name', { ...args })`；单参数对象命令统一命名 `req` / `input` / `patch` |
| 二进制 | 一律 base64 字符串；字段含义见各类型注释（终端 `data`、文件 `content`） |
| 时间 | 一律 Unix 毫秒 `number`（i64），前端负责本地化显示 |
| ID | UUIDv7 字符串（Rust 生成）；`sessionId` 例外，由前端 `crypto.randomUUID()` 生成 |
| 路径 | 远端路径一律 POSIX 绝对路径；本地路径用平台原生形式 |
| 空值 | 可空字段用 `null`，不用空字符串表示“无” |
| 错误 | 命令失败返回 `ApiError`（[§2](#2-错误模型)），不返回 `String` |
| 事件 | 名称 `域:动作`，payload 扁平对象且必带 `ts`；无“事件回放”，前端以命令结果为准 |
| 版本 | `app_info.apiVersion`；本文件当前 `1`。破坏性变更必须升版本（单应用，前后端同包发布） |
| 上限 | 单条 IPC 负载 ≤ 8MB；超过走文件传输/分片 |

**为什么 base64**：Tauri IPC 走 JSON，`Vec<u8>` 会被序列化成数字数组（体积 ×4~6）。终端输出与文件内容必须无损跨多字节 UTF-8，base64 是当前实现里最稳的选择。

## 2. 错误模型

```ts
export type ErrorCode =
  // 请求与通用
  | 'IPC_BAD_REQUEST' | 'CANCELLED' | 'INTERNAL'
  // SSH
  | 'SSH_TCP' | 'SSH_HANDSHAKE' | 'SSH_TIMEOUT' | 'SSH_CHANNEL' | 'SSH_CLOSED'
  | 'SSH_AUTH_FAILED' | 'SSH_AUTH_UNSUPPORTED' | 'SSH_AUTH_AGENT_EMPTY'
  | 'SSH_HOSTKEY_MISMATCH' | 'SSH_HOSTKEY_UNKNOWN'
  | 'SSH_RECONNECTING' | 'SSH_RECONNECT_EXHAUSTED'
  | 'SESSION_NOT_FOUND' | 'SESSION_EXISTS'
  // SFTP / 传输
  | 'SFTP_NOT_READY' | 'SFTP_IO' | 'SFTP_NOT_FOUND' | 'SFTP_PERMISSION'
  | 'SFTP_CONFLICT' | 'SFTP_TOO_LARGE' | 'SFTP_BINARY'
  | 'TRANSFER_CANCELLED' | 'TRANSFER_IO'
  // 转发 / 监控
  | 'FORWARD_BIND' | 'FORWARD_START' | 'FORWARD_REMOTE_DENIED'
  | 'MONITOR_UNSUPPORTED' | 'MONITOR_TIMEOUT'
  // 配置 / 存储
  | 'PROFILE_NOT_FOUND' | 'GROUP_NOT_FOUND' | 'GROUP_CYCLE' | 'DUPLICATE_NAME'
  | 'SECRET_NOT_FOUND' | 'KEYCHAIN_ERROR' | 'KEYCHAIN_UNAVAILABLE'
  | 'IMPORT_PARSE' | 'IMPORT_EMPTY' | 'IMPORT_VERSION'
  | 'DB_ERROR' | 'DB_LOCKED' | 'DB_MIGRATION'
  | 'INVALID_PATH' | 'PERMISSION_DENIED'

export interface ApiError {
  code: ErrorCode
  /** 面向用户的中文短句，可直接展示；不得包含密码/私钥/口令 */
  message: string
  /** 技术细节（stderr 摘要、错误链），仅用于“详情”折叠区与日志 */
  detail?: string
  /** 是否值得自动重试（网络类 true；认证/参数类 false） */
  retryable?: boolean
}
```

约定：

- 命令失败时前端 `catch (e)` 直接拿到上述对象（A 期是字符串，B 期首提交统一）。
- 长任务（导入、传输）启动成功后失败，通过对应的 `*:error` 事件报告，命令本身已返回。
- 错误文案由 Rust 侧生成（贴近原因），前端只做补充上下文，不重写语义。

## 3. 命令清单

> 阶段列：**A** = A 期已实现，**B** = B 期实现，**P1/P2** = 后续保留。

### 3.1 应用

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `app_info` | A | — | `AppInfo` |
| `app_reveal_data_dir` | P1 | `{ kind: 'data' \| 'logs' }` | `void` |

```ts
interface AppInfo {
  version: string          // 0.1.0
  apiVersion: number       // 1
  dbVersion: number        // PRAGMA user_version
  arch: string             // 'aarch64'
  os: string               // 'macos' | 'windows' | 'linux'
  tauriVersion: string
}
```

### 3.2 连接配置（Profile）

| 命令 | 阶段 | 请求 | 返回 | 说明 |
| --- | --- | --- | --- | --- |
| `profile_list` | B | `{ groupId?: Id \| null }` | `ConnProfile[]` | 省略 `groupId` = 全部；`null` = 仅未分组 |
| `profile_get` | B | `{ id }` | `ConnProfile` | 不含任何 secret |
| `profile_create` | B | `ProfileInput` | `ConnProfile` | 可同时带 `secret`（写钥匙串，不入库） |
| `profile_update` | B | `{ id, patch: Partial<ProfileInput> }` | `ConnProfile` | `secret` 为 `null` 表示删除已存凭据 |
| `profile_delete` | B | `{ ids: Id[] }` | `{ deleted: number }` | 事务内删库 + 尽力删钥匙串项 |
| `profile_duplicate` | B | `{ id, name?: string }` | `ConnProfile` | 不复制 secret（新配置需重输密码） |
| `profile_move` | B | `{ id, groupId: Id \| null, index: number }` | `void` | 拖拽排序/换组，单事务重排 |
| `profile_test` | B | `{ input?: ProfileInput, profileId?: Id }` | `TestResult` | 临时连接，不进会话表；**与正式连接使用相同 hostKeyPolicy（首次仍按 TOFU 记录指纹）**，仅不创建终端/SFTP/监控 |
| `profile_set_secret` | B | `{ profileId, kind: 'password' \| 'passphrase', value: string }` | `{ hasSecret: true }` | 只写不读 |
| `profile_clear_secret` | B | `{ profileId, kind }` | `void` | — |

```ts
type AuthType = 'password' | 'key' | 'agent'

interface ProfileInput {
  name: string
  groupId: Id | null
  host: string
  port: number
  username: string
  authType: AuthType
  keyPath: string | null
  agentIdentity: string | null
  term?: string
  charset?: string
  keepaliveSecs?: number
  connectTimeoutSecs?: number
  jumpProfileId?: Id | null     // P1 跳板机
  tags?: string[]
  color?: string | null
  notes?: string | null
  /** 写入钥匙串；仅创建/更新时允许，读取永不返回 */
  secret?: { kind: 'password' | 'passphrase'; value: string } | null
}

interface ConnProfile {
  id: Id
  groupId: Id | null
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  keyPath: string | null
  keyFingerprint: string | null    // SHA256:...，创建时后台计算，用于 UI 展示
  agentIdentity: string | null
  hasPassword: boolean
  hasPassphrase: boolean
  term: string
  charset: string
  keepaliveSecs: number
  connectTimeoutSecs: number
  jumpProfileId: Id | null
  tags: string[]
  color: string | null
  notes: string | null
  sortOrder: number
  lastConnectedAt: UnixMs | null
  createdAt: UnixMs
  updatedAt: UnixMs
}

interface TestResult {
  ok: boolean
  latencyMs?: number
  banner?: string          // 服务端登录 banner（截断 2KB）
  authMethods?: string[]   // 服务端支持的认证方式（握手返回时可用）
  error?: ApiError         // ok=false 时必填
}
```

校验规则（Rust 侧强制执行，前端同步做即时校验）：

- `name` 1–64 字符；`host` 非空、≤255；`port` 1–65535；`username` 非空、≤64。
- `authType=key` 时 `keyPath` 必填；`agent` 时 `keyPath/agentIdentity` 可空。
- `keepaliveSecs` 0–600（0 = 关闭）；`connectTimeoutSecs` 5–120。
- 设置 `secret` 但 `authType` 不匹配（如 password 类型却存 passphrase）→ `IPC_BAD_REQUEST`。

### 3.3 分组（Group）

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `group_list` | B | — | `Group[]`（扁平，`parentId` 组树） |
| `group_create` | B | `{ parentId: Id \| null, name }` | `Group` |
| `group_update` | B | `{ id, patch: { name?, icon? } }` | `Group` |
| `group_delete` | B | `{ id, childrenStrategy: 'move-to-parent' \| 'delete-all' }` | `{ deletedGroups, movedProfiles, deletedProfiles }` |
| `group_move` | B | `{ id, parentId: Id \| null, index }` | `void` |

```ts
interface Group {
  id: Id
  parentId: Id | null
  name: string
  icon: string | null
  sortOrder: number
  profileCount: number      // 仅直接子项
  createdAt: UnixMs
  updatedAt: UnixMs
}
```

- 同级重名 → `DUPLICATE_NAME`（大小写敏感，首尾空格已 trim）。
- 拖拽形成环（把自己拖进后代）→ `GROUP_CYCLE`。
- `delete-all` 必须由前端二次确认；默认 `move-to-parent`。

### 3.4 导入 / 导出

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `import_preview` | B | `ImportRequest` | `ImportPreview` |
| `import_apply` | B | `ImportRequest & { options: ImportOptions }` | `ImportResult` |
| `export_profiles` | B | `ExportRequest` | `{ path: string, count: number }` |

```ts
type ImportSource = 'finalshell-dir' | 'finalshell-json' | 'nestterm-json'

interface ImportRequest {
  source: ImportSource
  path: string                 // 目录或文件绝对路径
  defaultGroupId?: Id | null
}

interface ImportPreview {
  source: ImportSource
  profiles: number
  groups: number
  duplicates: number           // host+port+username 已存在
  conflicts: number            // 名称冲突
  warnings: string[]           // 解析告警，逐条可复制
}

interface ImportOptions {
  conflict: 'skip' | 'overwrite' | 'rename'
  targetGroupId?: Id | null
  includeSecrets: boolean       // 仅 nestterm-json 且文件含加密凭据时有效
  passphrase?: string           // 解密导出凭据的口令
}

interface ImportResult {
  createdProfiles: Id[]
  createdGroups: Id[]
  skipped: number
  warnings: string[]
}
```

- 解析必须**容错**：单条失败只记 warning，不中断整批。
- FinalShell 字段映射以真实导出样本为准（B 期第 0 步：先用脱敏样本做映射表），映射表见 [import-mapping.md](import-mapping.md)。
- 导入过程通过 `import:progress` 事件上报；`import_preview` 不写库。

### 3.5 SSH 会话

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `ssh_connect` | A（扩展 B） | `{ req: ConnectRequest }` | `ConnectResult` |
| `ssh_write` | A | `{ sessionId, data: B64 }` | `void` |
| `ssh_resize` | A | `{ sessionId, cols, rows }` | `void` |
| `ssh_disconnect` | A | `{ sessionId }` | `void` |
| `ssh_ping` | P1 | `{ sessionId }` | `{ latencyMs: number }` |

```ts
interface ConnectRequest {
  sessionId: string
  /** 已保存配置：host/port/username 从库里取，auth 可用 stored */
  profileId?: Id
  /** 临时连接（快速连接/测试），与 profileId 二选一 */
  target?: { host: string; port: number; username: string }
  auth: AuthInput
  term?: string                 // 默认 xterm-256color
  cols: number
  rows: number
  hostKeyPolicy?: 'tofu' | 'strict'   // 默认 tofu；strict 拒绝未知主机
}

type AuthInput =
  | { method: 'password'; source: 'inline'; password: string }
  | { method: 'password'; source: 'stored' }               // 需 profileId
  | { method: 'key'; source: 'inline'; keyPath: string; passphrase?: string }
  | { method: 'key'; source: 'stored'; keyPath: string }   // 口令从钥匙串取（可无）
  | { method: 'agent'; identityPath?: string }

interface ConnectResult {
  sessionId: string
  authMethod: AuthType
  serverFingerprint: string      // SHA256:...
  latencyMs: number              // TCP+握手+认证耗时
}
```

行为约定：

- `profileId` 与 `target` 同时给 → `IPC_BAD_REQUEST`；都不给 → 同上。
- `sessionId` 已存在 → `SESSION_EXISTS`；**自动重连沿用同一 `sessionId`**（标签、终端、SFTP 句柄不换主），只有用户主动重建标签时才生成新 ID。
- 连接前 `hostKeyPolicy=strict` 且主机未知 → `SSH_HOSTKEY_UNKNOWN`，前端引导用户「信任并保存」。
- 密码/私钥口令**只进不出**；`ConnectResult` 不含任何凭据。
- 连接成功后自动：启动 keepalive、探测 `$TERM`/cwd、按需开 SFTP/monitor/forward（由前端显式调用，不隐式做事）。
- `ssh_disconnect` 幂等；关闭时先 `eof` 再 `close`，2s 未完成则强制丢弃。

### 3.6 SFTP（复用同一 SSH 连接）

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `sftp_open` | B | `{ sessionId }` | `{ sftpSessionId, home }` |
| `sftp_close` | B | `{ sessionId }` | `void` |
| `sftp_list` | B | `{ sessionId, path, showHidden?: boolean }` | `SftpListing` |
| `sftp_stat` | B | `{ sessionId, path }` | `SftpEntry` |
| `sftp_realpath` | B | `{ sessionId, path }` | `{ path }` |
| `sftp_mkdir` | B | `{ sessionId, path, mode?: number }` | `void` |
| `sftp_rename` | B | `{ sessionId, from, to, overwrite?: boolean }` | `void` |
| `sftp_delete` | B | `{ sessionId, paths: string[], recursive: boolean }` | `{ deleted: number }` |
| `sftp_chmod` | B | `{ sessionId, path, mode: number }` | `void` |
| `sftp_read_file` | B | `{ sessionId, path, maxBytes?: number }` | `SftpFileContent` |
| `sftp_write_file` | B | `WriteFileRequest` | `{ size, mtimeMs, mode }` |
| `sftp_drag_out` | B | `{ sessionId, remotePath }` | `()`；macOS 原生拖出，drop 时才下载 |
| `sftp_upload` | B | `UploadRequest` | `{ transferIds: string[] }` |
| `sftp_download` | B | `DownloadRequest` | `{ transferIds: string[] }` |
| `sftp_cancel_transfer` | B | `{ transferId }` | `void` |
| `sftp_space` | P1 | `{ sessionId, path }` | `{ totalKb, freeKb }` |

```ts
interface SftpEntry {
  name: string
  path: string                  // 绝对路径（目录不带尾斜杠）
  kind: 'file' | 'dir' | 'symlink' | 'other'
  size: number                  // 字节；目录为 0
  mtimeMs: number
  mode: number                  // 权限位（0o644 等）
  uid?: number
  gid?: number
  linkTarget?: string           // kind=symlink 时的目标（解析失败为空）
}

interface SftpListing {
  path: string
  parent: string | null         // 根目录为 null
  entries: SftpEntry[]
  truncated?: boolean           // 单目录 > 5000 项时截断
}

interface SftpFileContent {
  path: string
  content: B64
  size: number
  mtimeMs: number
  mode: number
  isBinary: boolean             // 前 8KB 含 NUL 字节
  truncated: boolean            // 超过 maxBytes（默认 2MB）
}

interface WriteFileRequest {
  sessionId: string
  path: string
  content: B64
  /** 冲突检测：与读取时一致才允许覆盖；省略则不做检查 */
  expectedMtimeMs?: number
  expectedSize?: number
  createBackup?: boolean        // 服务器上写 <path>.nestterm.bak
}

interface UploadRequest {
  sessionId: string
  localPaths: string[]
  remoteDir: string
  conflictPolicy: 'ask' | 'overwrite' | 'skip' | 'rename'
  preserveMode?: boolean        // 默认 false（文件 0644 / 目录 0755）
}

interface DownloadRequest {
  sessionId: string
  remotePaths: string[]
  localDir: string
  conflictPolicy: 'ask' | 'overwrite' | 'skip' | 'rename'
  preserveMode?: boolean        // 默认 true（下载保留远端权限）
}
```

实现约定：

- `sftp_open` 必须在 SSH 已连接后调用；重复调用返回同一 `sftpSessionId`（幂等）。
- 所有 SFTP 操作在 Rust 侧串行化（`russh-sftp` 单会话多请求由库排队），前端可并发调用但需按 `path` 校验返回值是否过期。
- `sftp_write_file` 原子落盘：同目录写 `.nestterm.<rand>.tmp` → `rename` 覆盖；保留原 `mode`。
- 上传/下载的 `conflictPolicy='ask'` 在**启动前**由前端完成冲突列举（调用 `sftp_stat` 逐个探测），Rust 只接受确定的策略。
- 符号链接：列表展示 `linkTarget`；下载解引用目标；上传解引用本地链接（不创建远端软链）。
- `mode` 为十进制整数（`0o755 = 493`），前端用八进制字符串展示与输入。

### 3.7 端口转发

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `forward_list` | B | `{ profileId?: Id }` | `ForwardRule[]` |
| `forward_save` | B | `{ id?: Id, rule: ForwardRuleInput }` | `ForwardRule` |
| `forward_delete` | B | `{ id }` | `void` |
| `forward_start` | B | `{ ruleId, sessionId }` | `{ forwardId }` |
| `forward_stop` | B | `{ forwardId }` | `void` |

```ts
type ForwardKind = 'local' | 'remote' | 'dynamic'

interface ForwardRuleInput {
  profileId: Id
  name: string
  kind: ForwardKind
  bindHost: string              // 默认 127.0.0.1
  bindPort: number
  targetHost: string | null     // dynamic 必为 null
  targetPort: number | null
  autoStart: boolean
}

interface ForwardRule extends ForwardRuleInput {
  id: Id
  createdAt: UnixMs
  updatedAt: UnixMs
}
```

- `-L`：监听 `bindHost:bindPort`，每个入站连接开一个 `channel_open_direct_tcpip`。
- `-R`：调用 `tcpip_forward(bindHost:bindPort)`；服务端 `AllowTcpForwarding`/`GatewayPorts` 拒绝时返回 `FORWARD_REMOTE_DENIED`（提示检查 sshd_config）。
- `-D`：本地 SOCKS5（无认证，仅 CONNECT），默认只绑 `127.0.0.1`；绑定非回环地址必须前端二次确认。
- 端口占用 → `FORWARD_BIND`，detail 含系统原文。
- 会话断开时所有 forward 自动 `stopped` 并发 `forward:status`。

### 3.8 服务器监控

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `monitor_start` | B | `{ sessionId, intervalMs?: number }` | `void` |
| `monitor_stop` | B | `{ sessionId }` | `void` |

- 同一 `sessionId` 重复 `monitor_start` 幂等（仅更新间隔）。
- 采样通过同一 SSH 连接上的独立 exec channel，不占用终端通道。
- 采集脚本与解析规则见 [§6.6](#66-监控采样)。
- 会话断开自动停止；前端切走标签仅暂停 UI 渲染，不频繁 stop/start。

### 3.9 主机密钥（known_hosts）

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `known_hosts_list` | B | — | `KnownHost[]` |
| `known_hosts_remove` | B | `{ id }` | `void` |

```ts
interface KnownHost {
  id: Id
  host: string
  port: number
  keyType: string          // ssh-ed25519 / rsa-sha2-256 ...
  fingerprint: string      // SHA256:...
  firstSeenAt: UnixMs
  lastSeenAt: UnixMs
}
```

### 3.10 设置

| 命令 | 阶段 | 请求 | 返回 |
| --- | --- | --- | --- |
| `settings_get` | B | — | `Settings`（已合并默认值） |
| `settings_update` | B | `{ patch: DeepPartial<Settings> }` | `Settings` |

```ts
interface Settings {
  theme: 'dark' | 'light'       // B 期只实现 dark；light 为 P1
  terminal: {
    fontFamily: string
    fontSize: number            // 11–18
    lineHeight: number          // 1.0–1.6
    scrollback: number          // 1000–50000
    cursorBlink: boolean
    cursorStyle: 'block' | 'bar' | 'underline'
  }
  behavior: {
    confirmOnCloseTab: boolean
    autoReconnect: boolean
    maxRetries: number          // 0–10，默认 5
    pasteWarnBytes: number      // 默认 65536
    editorMaxBytes: number      // 默认 2MB
  }
  sftp: {
    showHidden: boolean
    followTerminal: boolean
    preserveMode: boolean
    conflictPolicy: 'ask' | 'overwrite' | 'skip' | 'rename'
    downloadDir: string         // 默认系统下载目录
  }
  monitor: { intervalMs: number; enabledOnConnect: boolean }
  security: { hostKeyPolicy: 'tofu' | 'strict' }
  ui: {
    sidebarWidth: number
    dockWidth: number
    monitorHeight: number
    monitorCollapsed: boolean
    sidebarCollapsed: boolean
  }
}
```

### 3.11 P1 / P2 预留（本期不实现，仅占位）

| 命令 | 说明 |
| --- | --- |
| `snippet_list` / `snippet_save` / `snippet_delete` | 命令片段库 |
| `batch_execute` | 批量命令（多会话并发执行，返回逐会话结果） |
| `log_start` / `log_stop` / `log_list` / `log_read` | 终端日志与回放 |
| `replay_read` | asciinema cast 读取 |
| `ssh_ping` | 状态栏延迟 |

## 4. 事件清单

所有事件 payload 必带 `sessionId`（若有）与 `ts`。

| 事件 | payload | 频率 | 阶段 |
| --- | --- | --- | --- |
| `ssh:status` | `SshStatusEvent` | 状态变化时 | A |
| `ssh:data` | `SshDataEvent` | 8ms 合并 / ≤256KB 一批 | A |
| `ssh:hostkey` | `SshHostKeyEvent` | 每次握手 1 次 | A |
| `ssh:cwd` | `SshCwdEvent` | cwd 变化时（去重） | B |
| `transfer:progress` | `TransferProgressEvent` | ≥100ms 且进度变化 ≥1% | B |
| `transfer:done` | `TransferDoneEvent` | 每任务 1 次 | B |
| `monitor:sample` | `MonitorSampleEvent` | 每个采样周期 | B |
| `monitor:error` | `MonitorErrorEvent` | 失败时 + 每次重试 | B |
| `forward:status` | `ForwardStatusEvent` | 状态变化 / 每 5s 流量 | B |
| `import:progress` | `ImportProgressEvent` | 阶段/批量进度 | B |

```ts
type SessionState =
  | 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'error'

interface SshStatusEvent {
  sessionId: string
  state: SessionState
  message?: string
  code?: ErrorCode          // state=error 时
  attempt?: number          // reconnecting 时：第几次
  nextRetryMs?: number      // reconnecting 时：距下次重试
  ts: UnixMs
}

interface SshDataEvent {
  sessionId: string
  data: B64
  ts: UnixMs
}

interface SshHostKeyEvent {
  sessionId: string
  fingerprint: string
  keyType: string
  trusted: boolean          // 与库中记录一致
  learned: boolean          // 本次首次写入
  ts: UnixMs
}

interface TransferProgressEvent {
  transferId: string
  sessionId: string
  direction: 'upload' | 'download'
  localPath: string
  remotePath: string
  bytes: number
  totalBytes: number
  speedBps: number
  filesDone?: number
  filesTotal?: number
  ts: UnixMs
}

interface TransferDoneEvent {
  transferId: string
  sessionId: string
  status: 'completed' | 'cancelled' | 'failed'
  error?: ApiError
  ts: UnixMs
}

interface MonitorSampleEvent {
  sessionId: string
  sample: MonitorSample
  ts: UnixMs
}

interface MonitorSample {
  cpu: { usagePct: number; cores: number; load1: number; load5: number; load15: number }
  mem: { totalKb: number; usedKb: number; availableKb: number; swapTotalKb: number; swapUsedKb: number }
  disks: Array<{ mount: string; fs: string; totalKb: number; usedKb: number; availKb: number; usedPct: number }>
  nets: Array<{ iface: string; rxBytes: number; txBytes: number; rxBps: number; txBps: number }>
  uptimeSec: number
  kernel: string
}

interface MonitorErrorEvent {
  sessionId: string
  code: ErrorCode
  message: string
  attempt: number
  ts: UnixMs
}

interface ForwardStatusEvent {
  forwardId: string
  ruleId: Id
  sessionId: string
  state: 'starting' | 'running' | 'stopped' | 'error'
  message?: string
  bytesIn: number
  bytesOut: number
  connections: number
  ts: UnixMs
}

interface ImportProgressEvent {
  taskId: string
  phase: 'parsing' | 'writing' | 'secrets'
  done: number
  total: number
  ts: UnixMs
}
```

**顺序不变量**（前端可依赖）：

1. `ssh:hostkey` 一定早于同会话的 `ssh:status(connected)`。
2. `ssh:status(connected)` 一定早于该会话第一条 `ssh:data`。
3. 同一会话的 `ssh:data` 严格有序，不重排、不合并跨批次（合并发生在批次内部）。
4. 终态 `disconnected|error` 之后不再有该 `sessionId` 的 `ssh:data`。
5. 每个 `transferId` 只有一条 `transfer:done`，且一定在最后一条 `progress` 之后。

## 5. 时序

### 5.1 建立连接（保存的密码）

```
UI                Rust                        远端 sshd
 │ ssh_connect(req) │                              │
 ├─────────────────►│ TCP connect ────────────────►│
 │                  │ ◄──────────── KEX ───────────│
 │                  │ check_server_key → DB known_hosts
 │ ◄ ssh:hostkey ───┤                              │
 │                  │ authenticate_password ──────►│
 │                  │ ◄───────── success ──────────│
 │                  │ channel_open_session         │
 │                  │ request_pty(true, cols, rows)│
 │                  │ request_shell(true)          │
 │ ◄─ ConnectResult ┤                              │
 │ ◄ ssh:status ────┤ connected                    │
 │                  │ ┌ read loop ── ssh:data ──►  │
 │ ssh_write ──────►│ │ data_bytes ───────────────►│
 │ ssh_resize ─────►│ └ window_change ────────────►│
```

### 5.2 断线自动重连（后端编排）

**为什么由后端编排**：重连不应依赖 UI 定时器（用户可能已切到别的标签）；且 `sessionId` 保持不变，SFTP/监控/转发无需换句柄。代价是会话期间需保留凭据（见下方安全约定）。

```
读写循环异常结束（EOF/Close/keepalive 超时/IO error）
   │
   ├─ 判定：autoReconnect && retryable && attempt < maxRetries && 凭据可用
   │        （认证失败 / 指纹变更 / 用户主动断开 → 不重连）
   │
   ├─ ssh:status{state:'reconnecting', attempt, nextRetryMs, code}
   │
   ├─ 退避：第 1 次立即；之后 1s → 2s → 4s → 8s（cap 30s，±20% jitter）
   │
   ├─ 重建：TCP + 握手 + 认证
   │         → request_pty（沿用最后已知 cols/rows）→ request_shell
   │         → 若此前 sftp_open 过：自动重开 SFTP
   │         → 若此前 monitor_start 过：自动恢复采样（沿用 interval）
   │         → 重启此前 running 的 forward 规则
   │
   ├─ 成功：ssh:status{connected, message:'已重新连接'}，attempt 归零
   │        前端在终端插入分隔线（后端不伪造 ssh:data 内容）
   │
   └─ 失败：继续退避；达到上限
            → ssh:status{error, code:'SSH_RECONNECT_EXHAUSTED'}，会话终止
```

- 重连期间对会话发起的命令（`ssh_write`/`sftp_*`）返回 `SSH_RECONNECTING`，前端禁用输入并显示遮罩提示。
- 用户点「断开」→ 取消退避，状态 `disconnected`，不再重试。
- 重连不重放历史输出；新 shell 的 cwd 由提示符重新解析。
- **安全**：`auth.source='stored'` 每次重试从钥匙串重读；`auth.source='inline'` 的密码以 `Zeroizing<Vec<u8>>` 仅缓存在该会话内存，会话终止即清零。设置项 `autoReconnect=false` 时完全不保留凭据。

### 5.3 终端 cwd → SFTP 跟随

```
远端输出 ──► ssh:data → xterm
                         └─ 提示符解析（user@host:~/path$）──► path 变化
                                                               │
前端 SftpPane：若 followTerminal && sftp 已打开
      └─ sftp_list(path) ──► 更新表格（带导航序号，丢弃过期响应）
```

> 2026-10-27：`ssh:cwd` / OSC 7 / 输入启发式已移除；cwd 只由前端提示符解析。

### 5.4 上传（含冲突处理）

```
前端收集 localPaths ──► 逐个 sftp_stat 探测冲突
   ├─ 有冲突 ──► 对话框（覆盖/跳过/重命名/全部应用）
   └─ sftp_upload(...) ──► transferIds
          └─ transfer:progress ≥100ms ──► 托盘
                 └─ transfer:done ──► 刷新列表 + 清任务
```

### 5.5 监控采集

```
monitor_start(intervalMs)
   └─ 定时器（tokio interval，跳过重叠）
        └─ exec channel: sh -c "<采集脚本>"
             ├─ 成功 ──► 解析 /proc ──► monitor:sample
             └─ 失败 ──► monitor:error（退避 ×2，上限 30s；连续 5 次停止并提示）
```

## 6. 内部实现约束（影响契约行为）

### 6.1 会话表

`SshState` 持有 `session_id -> 写通道`（A 期已实现）。SFTP/monitor/forward 的运行时句柄挂在同一个会话条目下，`ssh_disconnect` 级联释放。

重连期间条目保留、状态为 `reconnecting`；除 `ssh_disconnect` 外的会话命令返回 `SSH_RECONNECTING`。重连成功后同一 `sessionId` 下的 SFTP/monitor/forward 句柄由后端自动重建，前端无需重新调用 `sftp_open`/`monitor_start`（但需按 `ssh:status(connected)` 刷新目录与面板）。

### 6.2 输出合并

8ms 窗口 / 256KB 阈值（A 期已实现）。**不得**把多个 `ssh:data` 合并成跨会话事件；批次内只允许同会话数据。

### 6.3 终端输入

前端将按键字符串按 UTF-8 编码为字节后 base64；Rust 不解释内容（不做命令解析），避免终端语义被中间层篡改。

### 6.4 cwd 解析

- OSC 7 格式：`ESC ] 7 ; file://<host>/<path> BEL` 或 `ESC \` 结尾；解析失败不影响字节转发。
- 启发式仅在用户按 `Enter` 后 150ms 触发，且需 `sftp_realpath` 校验；两次校验失败后本次会话停用启发式（避免误 `cd`）。
- `ssh:cwd` 事件去重：同路径不重复发。

### 6.5 SFTP 并发

- 单会话 SFTP 请求并发上限 4；超出排队。
- 传输并发默认 2（大文件），可在设置调 1–4。
- 目录列表 > 5000 项截断并标记 `truncated`，UI 提示「仅显示前 5000 项」。

### 6.6 监控采样

采集脚本一次 exec 输出带分隔符的纯文本，Rust 解析（不依赖 JSON 工具）：

```sh
echo '##cpu'; cat /proc/stat | head -1
echo '##mem'; cat /proc/meminfo
echo '##load'; cat /proc/loadavg
echo '##uptime'; cat /proc/uptime
echo '##net'; cat /proc/net/dev
echo '##df'; df -kP -x tmpfs -x devtmpfs -x squashfs
echo '##kernel'; uname -r
```

- CPU%：两次采样间 `idle/total` 差值计算；首次采样 `usagePct=0` 并标注预热。
- 超时 3s；脚本失败不重试超过 5 次。
- macOS/BSD 服务端：P0 仅返回 `MONITOR_UNSUPPORTED` 的降级字段（内核/负载/内存），完整支持 P1。

### 6.7 转发

- `-L`/`-D` 监听器由 Rust `tokio::net::TcpListener` 持有；`-R` 依赖 russh 的 `server_channel_open_forwarded_tcpip` 回调（需在 `client::Handler` 中实现）。
- 每个 forward 实例独立计数 `bytesIn/bytesOut/connections`，5s 汇总一次发 `forward:status`，避免逐字节事件。

### 6.8 安全

- 事件与错误中**禁止**出现密码、passphrase、私钥内容；日志对 `auth` 字段整体脱敏。
- `profile_get/list` 永不返回 secret。
- `ssh:data` 可能含敏感输出（用户 `cat` 了密钥），不做内容过滤，但**不落盘**（日志功能为显式开启，P1）。

## 7. 版本与兼容

- 单应用前后端同包发布，**不承诺跨版本 IPC 兼容**；但同一版本内命令/事件遵循本文。
- 新增字段必须可选（前端忽略未知字段）；删除/改名必须升 `apiVersion`。
- `dbVersion` 独立演进；`app_info` 同时暴露两者，便于“关于”页与故障排查。

## 8. 验收方式（B 期）

1. **契约单测（Rust）**：所有 `Serialize/Deserialize` 结构体做 JSON 往返测试；`ApiError` 序列化字段名断言。
2. **前端类型对齐**：`src/lib/types.ts` 与文档逐字段 review；引入 `pnpm typecheck`（npm 脚本）作为 CI 门槛。
3. **事件顺序测试**：用 russh 内置 server 起本地回环，断言 hostkey → connected → data → disconnected 顺序。
4. **错误码覆盖**：认证失败、指纹变更、端口拒绝、SFTP 无权限四类必须有集成测试或手工验收记录。

## 9. A → B 契约差异（必须按序处理）

| # | 项 | A 期现状 | B 期目标 | 处理 |
| --- | --- | --- | --- | --- |
| 1 | 命令错误 | `Result<T, String>` | `Result<T, ApiError>` | B 首提交统一；前端 `lib/ssh.ts` 适配 |
| 2 | `ssh_connect` 入参 | 扁平 `{host,port,username,password,cols,rows}` | `{profileId?, target?, auth, ...}` | 同步改 `App.vue` 与类型 |
| 3 | `ssh_connect` 返回 | `void` | `ConnectResult` | 前端可忽略返回值，无破坏 |
| 4 | 会话状态 | `connecting/connected/disconnected/error` | 增加 `reconnecting` | 前端状态机补齐 case |
| 5 | 事件 payload | 无 `ts` | 全部带 `ts` | 前端忽略即可，B 期加 |
| 6 | known_hosts | 读写 `~/.ssh/known_hosts` | 应用自有表（`known_hosts`） | 首次启动导入系统记录（只读），之后只写库 |
| 7 | `app_info` | `{version,arch,os}` | 增加 `apiVersion/dbVersion/tauriVersion` | 向后兼容 |
| 8 | 终端字节字段 | `data`（base64） | 保持 `data`（base64） | **不改名**，避免无谓迁移 |
| 9 | 主机密钥策略 | 固定 TOFU | 增加 `strict` 可选 | 默认行为不变 |
| 10 | 前端 sessionId 生命周期 | 收到 `disconnected`/`error` 即清空 | `reconnecting` 期间保留；仅终态清空 | 改 `App.vue` 状态机（否则重连后 UI 会丢会话） |

> 记忆点：**B 期第一件事不是写新功能，而是把 #1/#2/#4 落地**，否则后续所有命令都会带着旧错误结构返工。

## 10. B1 / B2a / B2b-1 实现偏差（临时）

已落地：B1（加密存储、配置 CRUD、多标签终端、密码 + 私钥）、B2a（SFTP 浏览/编辑、OSC 7 cwd、ssh-agent unix）与 B2b-1（文件上传/下载/进度/取消/冲突策略）。

| 项 | 契约 | 现状 | 计划 |
| --- | --- | --- | --- |
| `profile_list` | 支持 `{groupId}` 过滤 | 返回全量；前端本地分组 | B2b-2 |
| `profile_update` | `patch: Partial<ProfileInput>` | 全量 `ProfileInput`；`secret` 省略 = 保留原凭据 | B2b-2 回写文档或改 patch |
| `group_update` | `{name?, icon?}` | 仅 `name` | P1（图标随主题） |
| `ssh-agent` | P0 认证方式 | unix 已实现（`SSH_AUTH_SOCK`）；Windows 返回 `SSH_AUTH_UNSUPPORTED` | 二期补 Pageant |
| SFTP 浏览 | §3.6 | `open/close/list/stat/realpath/mkdir/rename/delete/chmod/read_file/write_file` 已实现，并于 Ubuntu 24.04 实机自检通过 | — |
| 监控 | `monitor_sample` | Linux `/proc` 采样已实现并实机验证 | — |
| 端口转发 | `forward_*` + direct-tcpip / tcpip-forward | local/dynamic/remote 均已实现并实机验证 | — |
| SFTP 传输 | `sftp_upload/download` + `transfer:*` | 文件与目录递归上传/下载、进度/取消/冲突策略均已实现并实机验证；下载目录选择器未实现 | B2b-2b |
| cwd 跟随 | shell 提示符 | 已实现提示符解析、导航序号、SFTP 列目录容错；OSC 7/启发式已删除 | — |
| 自动重连 | 后端编排，沿用 sessionId | 已实现后端编排：最多 3 次，SFTP 重开，转发规则恢复 | — |
| 会话恢复 | `sessions_save/saved` | 打开 profile 列表持久化到 settings KV | — |
| 导入 / 导出 | P0 | macOS FinalShell 4.6.5 本地导入、NestTerm JSON 导出/导入闭环均已实现并实测 | — |
| known_hosts 旧数据导入 | 首次从 `~/.ssh/known_hosts` 迁移 | 启动时幂等迁移，仅限已有连接主机；支持 hashed host；冲突不覆盖 | — |
| `ssh_ping` | P1 | 未实现 | 二期 |

新增 `ssh_hint_cwd` 命令（SFTP 打开时提供 home/当前目录，供启发式解析相对路径）。

已按契约落地的部分：`ApiError` 全命令统一、`ConnectResult`、事件 `ts`、`ssh:hostkey.keyType`、`hostKeyPolicy=tofu|strict`、`profile_set_secret/clear_secret`、`settings_*`、`known_hosts_*`、`sftp_upload/download/cancel_transfer`、`finalshell_preview/import`、`nestterm_export`、`nestterm_import`。
