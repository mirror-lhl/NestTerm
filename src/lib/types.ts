/**
 * IPC 契约类型（前端视角）。
 * 与 src-tauri/src/db/models.rs、api/*.rs、ssh/commands.rs 一一对应。
 */

export type Id = string
export type UnixMs = number
export type ErrorCode = string

export interface ApiError {
  code: ErrorCode
  message: string
  detail?: string
  retryable: boolean
}

export interface AppInfo {
  version: string
  apiVersion: number
  dbVersion: number
  arch: string
  os: string
  tauriVersion: string
  startupError?: ApiError
  /** 仅 debug 构建：联调自动化。 */
  dev?: {
    autoConnect?: string | null
    initialCommand?: string | null
    autoConnectPassword?: string | null
    sftpSelftest?: boolean
    transferSelftest?: boolean
    transferLocalPath?: string | null
    transferRemoteDir?: string | null
    transferDownloadDir?: string | null
    transferCancelLocal?: string | null
    importTest?: boolean
    exportTest?: boolean
    exportPath?: string | null
    jsonImportPath?: string | null
    forwardTest?: boolean
    monitorPanel?: boolean
    settingsPanel?: boolean
  }
}

// ---------------- 连接配置 ----------------

export type AuthType = 'password' | 'key' | 'agent'

export interface Group {
  id: Id
  parentId: Id | null
  name: string
  icon: string | null
  sortOrder: number
  profileCount: number
  createdAt: UnixMs
  updatedAt: UnixMs
}

export interface ConnProfile {
  id: Id
  groupId: Id | null
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  keyPath: string | null
  keyFingerprint: string | null
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

export interface SecretInput {
  kind: 'password' | 'passphrase'
  value: string
}

export interface ProfileInput {
  name: string
  groupId: Id | null
  host: string
  port: number
  username: string
  authType: AuthType
  keyPath?: string | null
  agentIdentity?: string | null
  term?: string
  charset?: string
  keepaliveSecs?: number
  connectTimeoutSecs?: number
  jumpProfileId?: Id | null
  tags?: string[]
  color?: string | null
  notes?: string | null
  /** 写钥匙串；读取时永不返回。 */
  secret?: SecretInput | null
}

export interface DeleteProfilesResult {
  deleted: number
}

// ---------------- SSH ----------------

export type SshStatusState =
  | 'idle'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'disconnected'
  | 'error'

export type CredentialSource = 'inline' | 'stored'

export type AuthInput =
  | { method: 'password'; source: 'inline'; password: string }
  | { method: 'password'; source: 'stored' }
  | { method: 'key'; source: 'inline'; keyPath: string; passphrase?: string }
  | { method: 'key'; source: 'stored'; keyPath: string }
  | { method: 'agent'; identityPath?: string }

export interface ConnectRequest {
  sessionId: string
  profileId?: Id
  target?: { host: string; port: number; username: string }
  auth?: AuthInput
  term?: string
  cols: number
  rows: number
  hostKeyPolicy?: 'tofu' | 'strict'
}

export interface ConnectResult {
  sessionId: string
  authMethod: AuthType
  serverFingerprint: string
  latencyMs: number
}

export interface SshDataPayload {
  sessionId: string
  data: string
  ts: UnixMs
}

export interface SshStatusPayload {
  sessionId: string
  state: SshStatusState
  message?: string | null
  ts: UnixMs
}

export interface SshHostKeyPayload {
  sessionId: string
  fingerprint: string
  keyType: string
  trusted: boolean
  learned: boolean
  ts: UnixMs
}

// ---------------- SFTP ----------------

export interface SftpEntry {
  name: string
  path: string
  kind: 'file' | 'dir' | 'symlink' | 'other'
  size: number
  mtimeMs: number
  mode: number
  uid?: number
  gid?: number
}

export interface SftpListing {
  path: string
  parent: string | null
  entries: SftpEntry[]
  truncated: boolean
}

export interface SftpFileContent {
  path: string
  content: string
  size: number
  mtimeMs: number
  mode: number
  isBinary: boolean
  truncated: boolean
}

// ---------------- 传输 ----------------

export interface TransferProgressPayload {
  transferId: string
  sessionId: string
  direction: 'upload' | 'download'
  localPath: string
  remotePath: string
  bytes: number
  totalBytes: number
  speedBps: number
  filesDone: number
  filesTotal: number
  ts: UnixMs
}

export interface TransferDonePayload {
  transferId: string
  sessionId: string
  status: 'completed' | 'cancelled' | 'failed'
  error?: ApiError
  ts: UnixMs
}

export interface TransferStartResult {
  transferIds: string[]
  skipped: string[]
  localDir?: string
}

export interface KnownHost {
  id: string
  host: string
  port: number
  keyType: string
  fingerprint: string
  firstSeenAt: number
  lastSeenAt: number
}
