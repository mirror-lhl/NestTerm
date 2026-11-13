import { reactive } from 'vue'
import { errorMessage, groupList, profileList } from '../lib/api'
import type { ConnProfile, Group } from '../lib/types'

/** 连接配置的轻量单例 store（不引入 Pinia）。 */
export const profileState = reactive({
  groups: [] as Group[],
  profiles: [] as ConnProfile[],
  loading: false,
  error: '',
})

export async function refreshProfiles(): Promise<void> {
  profileState.loading = true
  profileState.error = ''
  try {
    const [groups, profiles] = await Promise.all([groupList(), profileList()])
    profileState.groups = groups
    profileState.profiles = profiles
  } catch (error) {
    profileState.error = errorMessage(error)
  } finally {
    profileState.loading = false
  }
}

export function profilesInGroup(groupId: string | null): ConnProfile[] {
  return profileState.profiles
    .filter((profile) => profile.groupId === groupId)
    .sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name))
}

export function groupName(groupId: string | null): string {
  if (!groupId) return '未分组'
  return profileState.groups.find((group) => group.id === groupId)?.name ?? '未分组'
}

export function findProfile(id: string | null | undefined): ConnProfile | undefined {
  if (!id) return undefined
  return profileState.profiles.find((profile) => profile.id === id)
}
