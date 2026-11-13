<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ConnProfile, Group } from '../lib/types'
import { profileState, profilesInGroup } from '../stores/profiles'
import ProfileItem from './ProfileItem.vue'

const emit = defineEmits<{
  open: [profile: ConnProfile]
  edit: [profile: ConnProfile]
  duplicate: [profile: ConnProfile]
  remove: [profile: ConnProfile]
  'new-profile': [groupId: string | null]
  'new-group': [parentId: string | null]
  'rename-group': [group: Group]
  'remove-group': [group: Group]
}>()

const UNGROUPED = '__ungrouped__'

const query = ref('')
const collapsed = ref<Set<string>>(new Set())
const searchInput = ref<{ focus: () => void } | null>(null)

function focusSearch(): void {
  searchInput.value?.focus()
}

defineExpose({ focusSearch })

function toggle(key: string): void {
  const next = new Set(collapsed.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  collapsed.value = next
}

const sortedGroups = computed(() =>
  [...profileState.groups].sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name)),
)

const childrenOf = computed(() => {
  const map = new Map<string | null, Group[]>()
  for (const group of sortedGroups.value) {
    const list = map.get(group.parentId) ?? []
    list.push(group)
    map.set(group.parentId, list)
  }
  return map
})

interface FlatGroup {
  group: Group
  depth: number
}

const flatGroups = computed<FlatGroup[]>(() => {
  const out: FlatGroup[] = []
  const walk = (parentId: string | null, depth: number) => {
    for (const group of childrenOf.value.get(parentId) ?? []) {
      out.push({ group, depth })
      if (!collapsed.value.has(group.id)) walk(group.id, depth + 1)
    }
  }
  walk(null, 0)
  return out
})

const ungrouped = computed(() => profilesInGroup(null))

const searchResults = computed<ConnProfile[] | null>(() => {
  const keyword = query.value.trim().toLowerCase()
  if (!keyword) return null
  return profileState.profiles
    .filter((profile) =>
      [profile.name, profile.host, profile.username, ...profile.tags]
        .join(' ')
        .toLowerCase()
        .includes(keyword),
    )
    .sort((a, b) => a.name.localeCompare(b.name))
})

function groupProfileCount(groupId: string): number {
  return profilesInGroup(groupId).length
}

function groupHasContent(group: Group): boolean {
  if (groupProfileCount(group.id) > 0) return true
  return (childrenOf.value.get(group.id) ?? []).length > 0
}

function onGroupCommand(command: string, group: Group): void {
  if (command === 'new-profile') emit('new-profile', group.id)
  else if (command === 'rename') emit('rename-group', group)
  else if (command === 'remove') emit('remove-group', group)
}
</script>

<template>
  <div class="session-tree">
    <div class="tree-search">
      <el-input
        ref="searchInput"
        v-model="query"
        size="small"
        clearable
        placeholder="搜索连接 / 主机 / 标签"
      />
    </div>

    <div class="tree-scroll">
      <template v-if="searchResults">
        <div v-if="searchResults.length === 0" class="tree-empty">
          没有匹配的连接
        </div>
        <ProfileItem
          v-for="profile in searchResults"
          :key="profile.id"
          :profile="profile"
          :group-label="profileState.groups.find((g) => g.id === profile.groupId)?.name"
          @open="emit('open', profile)"
          @edit="emit('edit', profile)"
          @duplicate="emit('duplicate', profile)"
          @remove="emit('remove', profile)"
        />
      </template>

      <template v-else>
        <div v-if="profileState.profiles.length === 0 && profileState.groups.length === 0" class="tree-empty">
          <p>还没有连接</p>
          <el-button size="small" type="primary" @click="emit('new-profile', null)">新建连接</el-button>
          <el-button size="small" @click="emit('new-group', null)">新建分组</el-button>
        </div>

        <template v-for="node in flatGroups" :key="node.group.id">
          <div
            class="group-row"
            :style="{ paddingLeft: `${node.depth * 12 + 6}px` }"
            @click="toggle(node.group.id)"
          >
            <span class="caret">
              {{ groupHasContent(node.group) ? (collapsed.has(node.group.id) ? '▸' : '▾') : '·' }}
            </span>
            <span class="group-name">{{ node.group.name }}</span>
            <span class="group-count">{{ groupProfileCount(node.group.id) }}</span>
            <span class="spacer" />
            <button class="icon-btn" title="在此分组新建连接" @click.stop="emit('new-profile', node.group.id)">
              ＋
            </button>
            <el-dropdown
              trigger="click"
              @command="(command: string) => onGroupCommand(command, node.group)"
            >
              <button class="icon-btn" title="分组操作" @click.stop>⋯</button>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item command="new-profile">在此新建连接</el-dropdown-item>
                  <el-dropdown-item command="rename">重命名</el-dropdown-item>
                  <el-dropdown-item command="remove" divided>删除分组</el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>

          <template v-if="!collapsed.has(node.group.id)">
            <ProfileItem
              v-for="profile in profilesInGroup(node.group.id)"
              :key="profile.id"
              :profile="profile"
              :indent="node.depth + 1"
              @open="emit('open', profile)"
              @edit="emit('edit', profile)"
              @duplicate="emit('duplicate', profile)"
              @remove="emit('remove', profile)"
            />
          </template>
        </template>

        <div class="group-row ungrouped" @click="toggle(UNGROUPED)">
          <span class="caret">{{ ungrouped.length === 0 ? '·' : collapsed.has(UNGROUPED) ? '▸' : '▾' }}</span>
          <span class="group-name">未分组</span>
          <span class="group-count">{{ ungrouped.length }}</span>
          <span class="spacer" />
          <button class="icon-btn" title="新建连接" @click.stop="emit('new-profile', null)">＋</button>
        </div>

        <template v-if="!collapsed.has(UNGROUPED)">
          <ProfileItem
            v-for="profile in ungrouped"
            :key="profile.id"
            :profile="profile"
            :indent="1"
            @open="emit('open', profile)"
            @edit="emit('edit', profile)"
            @duplicate="emit('duplicate', profile)"
            @remove="emit('remove', profile)"
          />
        </template>

        <div v-if="profileState.error" class="tree-error">{{ profileState.error }}</div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.session-tree {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.tree-search {
  padding: 10px 12px 6px;
}

.tree-scroll {
  flex: 1;
  min-height: 0;
  padding: 2px 6px 12px;
  overflow-y: auto;
}

.group-row {
  display: flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  padding-right: 6px;
  border-radius: 6px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  user-select: none;
  cursor: default;
}

.group-row:hover {
  background: #121a22;
}

.group-row.ungrouped {
  margin-top: 6px;
  border-top: 1px solid #141c25;
  border-radius: 0;
}

.caret {
  width: 12px;
  flex: 0 0 12px;
  text-align: center;
  font-size: 10px;
  opacity: 0.7;
}

.group-name {
  overflow: hidden;
  color: var(--el-text-color-regular);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-count {
  color: var(--el-text-color-placeholder);
  font-size: 11px;
}

.spacer {
  flex: 1;
}

.icon-btn {
  padding: 0 4px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--el-text-color-secondary);
  font-size: 13px;
  line-height: 18px;
  cursor: pointer;
  opacity: 0;
}

.group-row:hover .icon-btn {
  opacity: 0.8;
}

.icon-btn:hover {
  background: #24303c;
  color: #3ddc97;
  opacity: 1;
}

.tree-empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  padding: 40px 12px;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  text-align: center;
}

.tree-empty p {
  margin: 0 0 4px;
}

.tree-error {
  margin: 12px 6px;
  color: #f26d78;
  font-size: 12px;
}
</style>
