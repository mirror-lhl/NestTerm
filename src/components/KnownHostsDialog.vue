<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { knownHostsList, knownHostsRemove } from '../lib/known_hosts'
import { errorMessage } from '../lib/api'
import type { KnownHost } from '../lib/types'

const props = defineProps<{ modelValue: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()
const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
})

const rows = ref<KnownHost[]>([])
const filter = ref('')
const loading = ref(false)

const filtered = computed(() => {
  const keyword = filter.value.trim().toLowerCase()
  if (!keyword) return rows.value
  return rows.value.filter(
    (row) =>
      row.host.toLowerCase().includes(keyword) ||
      row.fingerprint.toLowerCase().includes(keyword) ||
      row.keyType.toLowerCase().includes(keyword),
  )
})

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString()
}

async function load(): Promise<void> {
  loading.value = true
  try {
    rows.value = await knownHostsList()
  } catch (error) {
    ElMessage.error(errorMessage(error))
  } finally {
    loading.value = false
  }
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) void load()
  },
)

async function remove(row: KnownHost): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `删除 ${row.host}:${row.port} 的旧主机指纹？\n确认服务器重装/换机后再删除；删除后下次连接会重新信任。`,
      '删除主机指纹',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    await knownHostsRemove(row.id)
    await load()
    ElMessage.success('已删除，请重新连接')
  } catch (error) {
    ElMessage.error(errorMessage(error))
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="主机指纹" width="720px" append-to-body>
    <p class="hint">
      服务器重装或换机后，旧指纹会阻断连接。确认安全后删除对应记录，再重新连接。
    </p>
    <el-input v-model="filter" size="small" placeholder="搜索主机 / 指纹 / 类型" clearable />
    <el-table v-loading="loading" :data="filtered" size="small" height="360" class="table">
      <el-table-column prop="host" label="主机" min-width="150">
        <template #default="{ row }">{{ row.host }}:{{ row.port }}</template>
      </el-table-column>
      <el-table-column prop="keyType" label="类型" width="110" />
      <el-table-column prop="fingerprint" label="SHA256 指纹" min-width="260" show-overflow-tooltip />
      <el-table-column label="最近使用" width="170">
        <template #default="{ row }">{{ formatTime(row.lastSeenAt) }}</template>
      </el-table-column>
      <el-table-column label="" width="70">
        <template #default="{ row }">
          <el-button size="small" text type="danger" @click="remove(row as KnownHost)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
  </el-dialog>
</template>

<style scoped>
.hint {
  margin: 0 0 10px;
  color: #8fa4b9;
  font-size: 12px;
  line-height: 1.5;
}
.table {
  margin-top: 10px;
}
</style>
