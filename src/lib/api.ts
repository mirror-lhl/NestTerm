import { invoke } from '@tauri-apps/api/core'
import type {
  AppInfo,
  ConnProfile,
  DeleteProfilesResult,
  Group,
  ProfileInput,
} from './types'

/** 配置类 IPC 封装：与 src-tauri/src/api/* 一一对应。 */

export const appInfo = () => invoke<AppInfo>('app_info')

// ---- 分组 ----
export const groupList = () => invoke<Group[]>('group_list')
export const groupCreate = (parentId: string | null, name: string) =>
  invoke<Group>('group_create', { parentId, name })
export const groupUpdate = (id: string, name: string) => invoke<Group>('group_update', { id, name })
export const groupDelete = (id: string, childrenStrategy: 'move-to-parent' | 'delete-all' = 'move-to-parent') =>
  invoke<{ deletedGroups: number; movedProfiles: number; deletedProfiles: number }>('group_delete', {
    id,
    childrenStrategy,
  })
export const groupMove = (id: string, parentId: string | null, index: number) =>
  invoke<void>('group_move', { id, parentId, index })

// ---- 连接 ----
export const profileList = () => invoke<ConnProfile[]>('profile_list')
export const profileGet = (id: string) => invoke<ConnProfile>('profile_get', { id })
export const profileCreate = (input: ProfileInput) => invoke<ConnProfile>('profile_create', { input })
export const profileUpdate = (id: string, input: ProfileInput) =>
  invoke<ConnProfile>('profile_update', { id, input })
export const profileDelete = (ids: string[]) =>
  invoke<DeleteProfilesResult>('profile_delete', { ids })
export const profileDuplicate = (id: string, name?: string) =>
  invoke<ConnProfile>('profile_duplicate', { id, name })
export const profileSetSecret = (profileId: string, kind: 'password' | 'passphrase', value: string) =>
  invoke<{ hasSecret: boolean }>('profile_set_secret', { profileId, kind, value })
export const profileClearSecret = (profileId: string, kind: 'password' | 'passphrase') =>
  invoke<void>('profile_clear_secret', { profileId, kind })

// ---- 错误展示 ----

/** Tauri 的 Err 是序列化后的 ApiError 对象；这里统一转成可读文本。 */
export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as { message: unknown }).message)
  }
  return String(error)
}

export function errorDetail(error: unknown): string | undefined {
  if (error && typeof error === 'object' && 'detail' in error) {
    const detail = (error as { detail?: unknown }).detail
    return typeof detail === 'string' ? detail : undefined
  }
  return undefined
}
