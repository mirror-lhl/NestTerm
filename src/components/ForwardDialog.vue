<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  forwardRuleCreate,
  forwardRuleDelete,
  forwardRulesList,
  forwardRunning,
  forwardStart,
  forwardStop,
  type ForwardRule,
} from '../lib/forward'
import { errorMessage } from '../lib/api'

const props = defineProps<{
  modelValue: boolean
  sessionId: string | null
  profileId: string | null
}>()
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()

const visible = computed({
  get: () => props.modelValue,
  set: (value: boolean) => emit('update:modelValue', value),
})

const rules = ref<ForwardRule[]>([])
const running = ref<Set<string>>(new Set())
const busy = ref(false)
const form = reactive({
  name: '',
  kind: 'local' as 'local' | 'dynamic' | 'remote',
  bindHost: '127.0.0.1',
  bindPort: 8080,
  targetHost: '127.0.0.1',
  targetPort: 22,
})

async function refresh(): Promise<void> {
  if (!props.profileId) return
  try {
    rules.value = await forwardRulesList(props.profileId)
    running.value = new Set(await forwardRunning())
  } catch (error) {
    ElMessage.error(errorMessage(error))
  }
}

watch(
  () => [props.modelValue, props.profileId, props.sessionId],
  () => {
    if (props.modelValue) void refresh()
  },
)

async function create(): Promise<void> {
  if (!props.profileId) return
  busy.value = true
  try {
    await forwardRuleCreate({
      profileId: props.profileId,
      name: form.name.trim() || `${form.kind} ${form.bindPort}`,
      kind: form.kind,
      bindHost: form.bindHost.trim() || '127.0.0.1',
      bindPort: Number(form.bindPort),
      targetHost: form.kind === 'dynamic' ? null : form.targetHost.trim(),
      targetPort: form.kind === 'dynamic' ? null : Number(form.targetPort),
    })
    form.name = ''
    await refresh()
    ElMessage.success('转发规则已保存')
  } catch (error) {
    ElMessage.error(errorMessage(error))
  } finally {
    busy.value = false
  }
}

async function toggle(rule: ForwardRule): Promise<void> {
  if (!props.sessionId) {
    ElMessage.warning('会话未连接')
    return
  }
  busy.value = true
  try {
    if (running.value.has(rule.id)) {
      await forwardStop(rule.id)
      running.value.delete(rule.id)
      ElMessage.success('已停止')
    } else {
      await forwardStart(props.sessionId, rule.id)
      running.value.add(rule.id)
      ElMessage.success(`已监听 ${rule.bindHost}:${rule.bindPort}`)
    }
    running.value = new Set(running.value)
  } catch (error) {
    ElMessage.error(errorMessage(error))
  } finally {
    busy.value = false
  }
}

async function remove(rule: ForwardRule): Promise<void> {
  try {
    await ElMessageBox.confirm(`删除规则「${rule.name}」？`, '删除转发', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消',
    })
  } catch {
    return
  }
  try {
    await forwardRuleDelete(rule.id)
    running.value.delete(rule.id)
    running.value = new Set(running.value)
    await refresh()
  } catch (error) {
    ElMessage.error(errorMessage(error))
  }
}

function describe(rule: ForwardRule): string {
  if (rule.kind === 'dynamic') return 'SOCKS5'
  return `${rule.targetHost}:${rule.targetPort}`
}
</script>

<template>
  <el-dialog v-model="visible" title="端口转发" width="620px" append-to-body>
    <div v-if="!profileId" class="empty">当前连接是临时目标，保存连接后才能配置转发规则。</div>
    <template v-else>
      <div class="rule-list">
        <div v-for="rule in rules" :key="rule.id" class="rule">
          <div class="rule-main">
            <div class="rule-title">
              <span class="dot" :class="{ on: running.has(rule.id) }" />
              <strong>{{ rule.name }}</strong>
              <span class="tag">{{ rule.kind === 'dynamic' ? 'SOCKS5' : rule.kind === 'remote' ? '远端' : '本地' }}</span>
            </div>
            <div class="rule-desc">
              {{ rule.bindHost }}:{{ rule.bindPort }} → {{ describe(rule) }}
            </div>
          </div>
          <el-button size="small" :disabled="busy" @click="toggle(rule)">
            {{ running.has(rule.id) ? '停止' : '启动' }}
          </el-button>
          <el-button size="small" text type="danger" :disabled="busy" @click="remove(rule)">
            删除
          </el-button>
        </div>
        <div v-if="rules.length === 0" class="empty">还没有规则，添加一条本地或 SOCKS5 转发。</div>
      </div>

      <div class="form">
        <div class="form-title">新增规则</div>
        <div class="grid">
          <el-input v-model="form.name" placeholder="名称（可选）" />
          <el-select v-model="form.kind" style="width: 120px">
            <el-option label="本地 -L" value="local" />
            <el-option label="远端 -R" value="remote" />
            <el-option label="SOCKS5 -D" value="dynamic" />
          </el-select>
          <el-input v-model="form.bindHost" placeholder="监听地址" />
          <el-input-number v-model="form.bindPort" :min="1" :max="65535" controls-position="right" />
          <template v-if="form.kind !== 'dynamic'">
            <el-input v-model="form.targetHost" placeholder="目标主机" />
            <el-input-number v-model="form.targetPort" :min="1" :max="65535" controls-position="right" />
          </template>
        </div>
        <el-button type="primary" size="small" :loading="busy" @click="create">添加规则</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<style scoped>
.empty {
  padding: 14px;
  color: #7f8ea3;
  font-size: 12px;
  text-align: center;
}
.rule-list {
  max-height: 240px;
  overflow: auto;
  border: 1px solid #1c2733;
  border-radius: 6px;
}
.rule {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid #151e28;
}
.rule:last-child {
  border-bottom: none;
}
.rule-main {
  flex: 1;
  min-width: 0;
}
.rule-title {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #d7e2f0;
  font-size: 13px;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #4a5a6d;
}
.dot.on {
  background: #35c46b;
  box-shadow: 0 0 6px #35c46b;
}
.tag {
  padding: 1px 5px;
  border: 1px solid #2a3a4b;
  border-radius: 3px;
  color: #8fa3b8;
  font-size: 10px;
}
.rule-desc {
  margin-top: 3px;
  color: #7c8da1;
  font-family: 'JetBrains Mono', monospace;
  font-size: 11px;
}
.form {
  margin-top: 14px;
  padding-top: 12px;
  border-top: 1px solid #1c2733;
}
.form-title {
  margin-bottom: 8px;
  color: #aebccd;
  font-size: 12px;
}
.grid {
  display: grid;
  grid-template-columns: 1fr 120px 1fr 120px;
  gap: 8px;
  margin-bottom: 8px;
}
</style>
