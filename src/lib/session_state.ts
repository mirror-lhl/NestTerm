import { invoke } from '@tauri-apps/api/core'

export const sessionsSaved = () => invoke<string[]>('sessions_saved')
export const sessionsSave = (profileIds: string[]) =>
  invoke<void>('sessions_save', { profileIds })
