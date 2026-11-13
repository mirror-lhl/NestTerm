import { invoke } from '@tauri-apps/api/core'

export interface TerminalSettings {
  fontFamily: string
  fontSize: number
  lineHeight: number
  scrollback: number
  cursorBlink: boolean
  cursorStyle: string
}

export interface BehaviorSettings {
  confirmOnCloseTab: boolean
  autoReconnect: boolean
  restoreSessions: boolean
  maxRetries: number
  pasteWarnBytes: number
  editorMaxBytes: number
}

export interface SftpSettings {
  showHidden: boolean
  followTerminal: boolean
  preserveMode: boolean
  conflictPolicy: 'ask' | 'overwrite' | 'skip' | 'rename'
  downloadDir: string
}

export interface MonitorSettings {
  intervalMs: number
  enabledOnConnect: boolean
}

export interface SecuritySettings {
  hostKeyPolicy: 'tofu' | 'strict'
}

export interface UiSettings {
  sidebarWidth: number
  dockWidth: number
  monitorHeight: number
  monitorCollapsed: boolean
  sidebarCollapsed: boolean
}

export interface Settings {
  theme: 'dark' | 'light'
  terminal: TerminalSettings
  behavior: BehaviorSettings
  sftp: SftpSettings
  monitor: MonitorSettings
  security: SecuritySettings
  ui: UiSettings
}

export type SettingsPatch = Partial<Omit<Settings, 'theme'>> & { theme?: 'dark' | 'light' }

export const settingsGet = () => invoke<Settings>('settings_get')
export const settingsUpdate = (patch: SettingsPatch) =>
  invoke<Settings>('settings_update', { patch })
