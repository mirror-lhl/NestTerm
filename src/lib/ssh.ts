import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  ConnectRequest,
  ConnectResult,
  SshDataPayload,
  SshHostKeyPayload,
  SshStatusPayload,
} from './types'

/** SSH 会话 IPC 封装。 */

export const sshConnect = (req: ConnectRequest) => invoke<ConnectResult>('ssh_connect', { req })

export const sshWrite = (sessionId: string, data: string) =>
  invoke<void>('ssh_write', { sessionId, data })

export const sshResize = (sessionId: string, cols: number, rows: number) =>
  invoke<void>('ssh_resize', { sessionId, cols, rows })

export const sshDisconnect = (sessionId: string) =>
  invoke<void>('ssh_disconnect', { sessionId })

/** 列表结果被采用后，显式提交 authoritative cwd。 */
export const onSshData = (cb: (p: SshDataPayload) => void) =>
  listen<SshDataPayload>('ssh:data', (event) => cb(event.payload))

export const onSshStatus = (cb: (p: SshStatusPayload) => void) =>
  listen<SshStatusPayload>('ssh:status', (event) => cb(event.payload))

export const onSshHostKey = (cb: (p: SshHostKeyPayload) => void) =>
  listen<SshHostKeyPayload>('ssh:hostkey', (event) => cb(event.payload))

