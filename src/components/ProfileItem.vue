<script setup lang="ts">
import { computed } from 'vue'
import type { ConnProfile } from '../lib/types'
import { tabs } from '../stores/sessions'

const props = defineProps<{
  profile: ConnProfile
  indent?: number
  groupLabel?: string
}>()

const emit = defineEmits<{
  open: []
  edit: []
  duplicate: []
  remove: []
}>()

const status = computed(
  () => tabs.value.find((tab) => tab.profileId === props.profile.id)?.status ?? 'idle',
)

function onCommand(command: string): void {
  if (command === 'open') emit('open')
  else if (command === 'edit') emit('edit')
  else if (command === 'duplicate') emit('duplicate')
  else if (command === 'remove') emit('remove')
}
</script>

<template>
  <el-dropdown trigger="contextmenu" @command="onCommand">
    <div
      class="profile-row"
      :style="{ paddingLeft: `${(indent ?? 0) * 12 + 10}px` }"
      :class="{ active: status === 'connected' }"
      @dblclick="emit('open')"
    >
      <span class="status-dot" :class="`dot-${status}`" />
      <span class="pname">{{ profile.name }}</span>
      <span v-if="groupLabel" class="group-tag">{{ groupLabel }}</span>
      <span class="phost">{{ profile.host }}</span>
    </div>
    <template #dropdown>
      <el-dropdown-menu>
        <el-dropdown-item command="open">连接</el-dropdown-item>
        <el-dropdown-item command="edit">编辑</el-dropdown-item>
        <el-dropdown-item command="duplicate">复制配置</el-dropdown-item>
        <el-dropdown-item command="remove" divided>删除</el-dropdown-item>
      </el-dropdown-menu>
    </template>
  </el-dropdown>
</template>

<style scoped>
:deep(.el-dropdown) {
  display: block;
  width: 100%;
}

.profile-row {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding-right: 8px;
  border-radius: 6px;
  color: var(--el-text-color-regular);
  font-size: 12.5px;
  cursor: default;
  user-select: none;
}

.profile-row:hover {
  background: #141c25;
}

.profile-row.active {
  color: var(--el-text-color-primary);
}

.status-dot {
  flex: 0 0 7px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #3a4654;
}

.dot-connected {
  background: #3ddc97;
  box-shadow: 0 0 6px rgba(61, 220, 151, 0.6);
}

.dot-connecting {
  background: #6ea8fe;
}

.dot-reconnecting {
  background: #e6c07b;
}

.dot-error {
  background: #f26d78;
}

.dot-disconnected {
  background: #5c6b7a;
}

.pname {
  overflow: hidden;
  flex: 0 1 auto;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.phost {
  overflow: hidden;
  flex: 0 1 auto;
  margin-left: auto;
  color: var(--el-text-color-placeholder);
  font-size: 11px;
  font-family: 'SF Mono', Menlo, monospace;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-tag {
  flex: 0 0 auto;
  padding: 0 5px;
  border-radius: 4px;
  background: #1a2530;
  color: var(--el-text-color-secondary);
  font-size: 10px;
}
</style>
