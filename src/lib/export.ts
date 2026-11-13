import { invoke } from '@tauri-apps/api/core'

/** NestTerm 自有 JSON 导出（不含凭据）。 */

export interface ExportResult {
  path: string
  profiles: number
  groups: number
}

export const nesttermExport = (path?: string) =>
  invoke<ExportResult>('nestterm_export', { path: path ?? null })

export interface ImportResult {
  profilesCreated: number
  groupsCreated: number
  skipped: number
  warnings: string[]
}

export const nesttermImport = (path: string) =>
  invoke<ImportResult>('nestterm_import', { path })
