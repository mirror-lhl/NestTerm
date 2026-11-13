import { invoke } from '@tauri-apps/api/core'
import type { KnownHost } from './types'

export const knownHostsList = () => invoke<KnownHost[]>('known_hosts_list')
export const knownHostsRemove = (id: string) => invoke<void>('known_hosts_remove', { id })
