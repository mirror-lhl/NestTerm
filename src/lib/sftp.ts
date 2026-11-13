import { invoke } from '@tauri-apps/api/core'
import type { SftpEntry, SftpFileContent, SftpListing } from './types'

/** SFTP 命令封装：与会话共用同一条 SSH 连接。 */

export const sftpOpen = (sessionId: string) =>
  invoke<{ sftpSessionId: string; home: string }>('sftp_open', { sessionId })

export const sftpClose = (sessionId: string) => invoke<void>('sftp_close', { sessionId })

export const sftpList = (sessionId: string, path: string, showHidden = false) =>
  invoke<SftpListing>('sftp_list', { sessionId, path, showHidden })

export const sftpStat = (sessionId: string, path: string) =>
  invoke<SftpEntry>('sftp_stat', { sessionId, path })

export const sftpRealpath = (sessionId: string, path: string) =>
  invoke<string>('sftp_realpath', { sessionId, path })

export const sftpMkdir = (sessionId: string, path: string) =>
  invoke<void>('sftp_mkdir', { sessionId, path })

export const sftpRename = (sessionId: string, from: string, to: string) =>
  invoke<void>('sftp_rename', { sessionId, from, to })

export const sftpIsDir = (sessionId: string, path: string) =>
  invoke<boolean>('sftp_is_dir', { sessionId, path })

export const sftpDragOut = (sessionId: string, remotePath: string) =>
  invoke<void>('sftp_drag_out', { sessionId, remotePath })

export const sftpDelete = (sessionId: string, paths: string[], recursive: boolean) =>
  invoke<number>('sftp_delete', { sessionId, paths, recursive })

export const sftpChmod = (sessionId: string, path: string, mode: number) =>
  invoke<void>('sftp_chmod', { sessionId, path, mode })

export const sftpReadFile = (sessionId: string, path: string, maxBytes?: number) =>
  invoke<SftpFileContent>('sftp_read_file', { sessionId, path, maxBytes })

export const sftpWriteFile = (req: {
  sessionId: string
  path: string
  content: string
  expectedMtimeMs?: number
  expectedSize?: number
  createBackup?: boolean
}) => invoke<SftpEntry>('sftp_write_file', { req })
