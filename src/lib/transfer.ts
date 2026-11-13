import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  TransferDonePayload,
  TransferProgressPayload,
  TransferStartResult,
} from './types'

/** SFTP 文件传输命令与事件封装。 */

export const sftpUpload = (req: {
  sessionId: string
  localPaths: string[]
  remoteDir: string
  conflictPolicy?: 'ask' | 'overwrite' | 'skip' | 'rename'
  preserveMode?: boolean
}) => invoke<TransferStartResult>('sftp_upload', { req })

export const sftpDownload = (req: {
  sessionId: string
  remotePaths: string[]
  localDir?: string
  conflictPolicy?: 'ask' | 'overwrite' | 'skip' | 'rename'
  preserveMode?: boolean
}) => invoke<TransferStartResult>('sftp_download', { req })

export const sftpCancelTransfer = (transferId: string) =>
  invoke<void>('sftp_cancel_transfer', { transferId })

export const onTransferProgress = (cb: (p: TransferProgressPayload) => void) =>
  listen<TransferProgressPayload>('transfer:progress', (event) => cb(event.payload))

export const onTransferDone = (cb: (p: TransferDonePayload) => void) =>
  listen<TransferDonePayload>('transfer:done', (event) => cb(event.payload))
