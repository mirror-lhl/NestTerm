<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { errorMessage } from '../lib/api'
import { profileCreate, profileUpdate } from '../lib/api'
import type { AuthType, ConnProfile, Group, ProfileInput, SecretInput } from '../lib/types'

const props = defineProps<{
  visible: boolean
  groups: Group[]
  profile: ConnProfile | null
  presetGroupId: string | null
  mode: 'create' | 'edit' | 'connect'
}>()

const emit = defineEmits<{
  'update:visible': [visible: boolean]
  saved: [profile: ConnProfile]
  connect: [payload: { profile: ConnProfile; inlinePassword?: string; inlinePassphrase?: string }]
}>()

interface FormState {
  name: string
  groupId: string | null
  host: string
  port: number
  username: string
  authType: AuthType
  password: string
  keyPath: string
  passphrase: string
  savePassword: boolean
  savePassphrase: boolean
}

const form = reactive<FormState>({
  name: '',
  groupId: null,
  host: '',
  port: 22,
  username: 'root',
  authType: 'password',
  password: '',
  keyPath: '',
  passphrase: '',
  savePassword: true,
  savePassphrase: true,
})

const saving = ref(false)

const groupValue = computed({
  get: () => form.groupId ?? '__ungrouped__',
  set: (value: string) => {
    form.groupId = value === '__ungrouped__' ? null : value
  },
})
const errorText = ref('')

const title = computed(() => {
  if (props.mode === 'connect') return '连接配置'
  return props.profile ? '编辑连接' : '新建连接'
})

const passwordHint = computed(() =>
  props.profile?.hasPassword ? '已保存密码；留空表示不修改' : '密码将保存到系统钥匙串',
)

watch(
  () => [props.visible, props.profile?.id, props.presetGroupId] as const,
  ([visible]) => {
    if (!visible) return
    errorText.value = ''
    const profile = props.profile
    if (profile) {
      form.name = profile.name
      form.groupId = profile.groupId
      form.host = profile.host
      form.port = profile.port
      form.username = profile.username
      form.authType = profile.authType
      form.password = ''
      form.keyPath = profile.keyPath ?? ''
      form.passphrase = ''
      form.savePassword = true
      form.savePassphrase = true
    } else {
      form.name = ''
      form.groupId = props.presetGroupId
      form.host = ''
      form.port = 22
      form.username = 'root'
      form.authType = 'password'
      form.password = ''
      form.keyPath = ''
      form.passphrase = ''
      form.savePassword = true
      form.savePassphrase = true
    }
  },
  { immediate: true },
)

function close(): void {
  emit('update:visible', false)
}

function validate(connectAfter: boolean): string | null {
  if (!form.name.trim()) return '请填写连接名称'
  if (!form.host.trim()) return '请填写主机地址'
  if (!form.username.trim()) return '请填写用户名'
  if (form.port < 1 || form.port > 65535) return '端口需在 1–65535 之间'
  if (form.authType === 'key' && !form.keyPath.trim()) return '私钥认证必须填写私钥路径'
  if (connectAfter && form.authType === 'agent') {
    return 'ssh-agent 认证将在 B2 提供，请先使用密码或私钥'
  }
  if (connectAfter && form.authType === 'password') {
    const hasStored = Boolean(props.profile?.hasPassword)
    if (!form.password && !hasStored) return '请填写密码'
  }
  return null
}

async function submit(connectAfter: boolean): Promise<void> {
  const problem = validate(connectAfter)
  if (problem) {
    ElMessage.error(problem)
    return
  }

  let secret: SecretInput | undefined
  if (form.authType === 'password' && form.password && form.savePassword) {
    secret = { kind: 'password', value: form.password }
  } else if (form.authType === 'key' && form.passphrase && form.savePassphrase) {
    secret = { kind: 'passphrase', value: form.passphrase }
  }

  const input: ProfileInput = {
    name: form.name.trim(),
    groupId: form.groupId,
    host: form.host.trim(),
    port: form.port,
    username: form.username.trim(),
    authType: form.authType,
    keyPath: form.authType === 'key' ? form.keyPath.trim() : null,
    agentIdentity: null,
    term: props.profile?.term,
    charset: props.profile?.charset,
    // 仅在全量更新时携带原 tags/备注，避免编辑时丢失。
    tags: props.profile?.tags ?? [],
    color: props.profile?.color ?? null,
    notes: props.profile?.notes ?? null,
    secret,
  }

  saving.value = true
  errorText.value = ''
  try {
    const profile = props.profile
      ? await profileUpdate(props.profile.id, input)
      : await profileCreate(input)
    emit('saved', profile)

    if (connectAfter) {
      const inlinePassword =
        form.authType === 'password' && form.password && !form.savePassword
          ? form.password
          : undefined
      const inlinePassphrase =
        form.authType === 'key' && form.passphrase && !form.savePassphrase
          ? form.passphrase
          : undefined
      emit('connect', { profile, inlinePassword, inlinePassphrase })
    }
    close()
  } catch (error) {
    errorText.value = errorMessage(error)
    ElMessage.error(errorText.value)
  } finally {
    saving.value = false
  }
}

function groupOptions(): Array<{ id: string | null; label: string }> {
  const options: Array<{ id: string | null; label: string }> = [{ id: null, label: '未分组' }]
  const walk = (parentId: string | null, depth: number) => {
    for (const group of props.groups
      .filter((item) => item.parentId === parentId)
      .sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name))) {
      options.push({ id: group.id, label: `${'　'.repeat(depth)}${group.name}` })
      walk(group.id, depth + 1)
    }
  }
  walk(null, 0)
  return options
}
</script>

<template>
  <el-dialog
    :model-value="visible"
    :title="title"
    width="540px"
    :close-on-click-modal="false"
    destroy-on-close
    @update:model-value="emit('update:visible', $event)"
  >
    <el-alert v-if="errorText" type="error" :closable="false" show-icon :title="errorText" class="dialog-error" />

    <el-form label-position="top" @submit.prevent="submit(true)">
      <div class="row">
        <el-form-item label="名称" class="grow">
          <el-input v-model="form.name" placeholder="例如：生产 DB" maxlength="64" />
        </el-form-item>
        <el-form-item label="分组" class="grow">
          <el-select v-model="groupValue" class="w-full" placeholder="未分组">
            <el-option
              v-for="option in groupOptions()"
              :key="option.id ?? '__ungrouped__'"
              :label="option.label"
              :value="option.id ?? '__ungrouped__'"
            />
          </el-select>
        </el-form-item>
      </div>

      <div class="row">
        <el-form-item label="主机" class="grow">
          <el-input v-model="form.host" placeholder="192.0.2.10 或 example.com" autocomplete="off" />
        </el-form-item>
        <el-form-item label="端口" class="port">
          <el-input-number v-model="form.port" :min="1" :max="65535" controls-position="right" class="w-full" />
        </el-form-item>
      </div>

      <el-form-item label="用户名">
        <el-input v-model="form.username" autocomplete="off" />
      </el-form-item>

      <el-form-item label="认证方式">
        <el-radio-group v-model="form.authType">
          <el-radio-button value="password">密码</el-radio-button>
          <el-radio-button value="key">私钥</el-radio-button>
          <el-radio-button value="agent">ssh-agent</el-radio-button>
        </el-radio-group>
      </el-form-item>

      <template v-if="form.authType === 'password'">
        <el-form-item label="密码">
          <el-input
            v-model="form.password"
            type="password"
            show-password
            :placeholder="passwordHint"
            autocomplete="new-password"
          />
        </el-form-item>
        <el-checkbox v-model="form.savePassword">保存到系统钥匙串</el-checkbox>
      </template>

      <template v-else-if="form.authType === 'key'">
        <el-form-item label="私钥路径">
          <el-input v-model="form.keyPath" placeholder="~/.ssh/id_ed25519" />
        </el-form-item>
        <el-form-item label="私钥口令（没有留空）">
          <el-input
            v-model="form.passphrase"
            type="password"
            show-password
            :placeholder="props.profile?.hasPassphrase ? '已保存口令；留空表示不修改' : '可选'"
          />
        </el-form-item>
        <el-checkbox v-model="form.savePassphrase">保存口令到系统钥匙串</el-checkbox>
      </template>

      <el-alert
        v-else
        type="info"
        :closable="false"
        show-icon
        title="ssh-agent 认证将在 B2 提供；当前保存配置后仍可用密码/私钥连接"
        class="agent-tip"
      />
    </el-form>

    <template #footer>
      <el-button @click="close">取消</el-button>
      <el-button :loading="saving" @click="submit(false)">保存</el-button>
      <el-button type="primary" :loading="saving" @click="submit(true)">保存并连接</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.row {
  display: flex;
  gap: 10px;
}

.grow {
  flex: 1;
  min-width: 0;
}

.port {
  flex: 0 0 120px;
}

.w-full {
  width: 100%;
}

.w-full :deep(.el-input__wrapper) {
  width: 100%;
}

.dialog-error {
  margin-bottom: 12px;
}

.agent-tip {
  margin-top: 4px;
}
</style>
