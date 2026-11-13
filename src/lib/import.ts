import { invoke } from '@tauri-apps/api/core'

/** FinalShell 本地目录导入（4.6.x macOS 路径已实测）。 */

export interface FinalShellPreview {
  dir: string
  connections: number
  withPassword: number
  groupsDetected: number
  warnings: string[]
}

export interface FinalShellImportResult {
  groupId: string
  created: number
  skipped: number
  warnings: string[]
}

export const finalshellPreview = (dir?: string) =>
  invoke<FinalShellPreview>('finalshell_preview', { dir: dir ?? null })

export const finalshellImport = (dir?: string) =>
  invoke<FinalShellImportResult>('finalshell_import', { dir: dir ?? null })
