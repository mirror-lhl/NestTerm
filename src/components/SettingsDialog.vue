<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { appSettings, saveSettings } from '../stores/settings'
import { errorMessage } from '../lib/api'
import type { Settings } from '../lib/settings'

const props = defineProps<{ modelValue: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()
const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
})

const defaults: Settings = {
  theme: 'dark',
  terminal: {
    fontFamily: '"JetBrains Mono", "SF Mono", Menlo, Consolas, monospace',
    fontSize: 13,
    lineHeight: 1.25,
    scrollback: 10000,
    cursorBlink: true,
    cursorStyle: 'bar',
  },
  behavior: {
    confirmOnCloseTab: true,
    autoReconnect: true,
    restoreSessions: true,
    maxRetries: 5,
    pasteWarnBytes: 65536,
    editorMaxBytes: 2097152,
  },
  sftp: {
    showHidden: false,
    followTerminal: true,
    preserveMode: false,
    conflictPolicy: 'ask',
    downloadDir: '',
  },
  monitor: { intervalMs: 2000, enabledOnConnect: true },
  security: { hostKeyPolicy: 'tofu' },
  ui: {
    sidebarWidth: 300,
    dockWidth: 400,
    monitorHeight: 240,
    monitorCollapsed: false,
    sidebarCollapsed: false,
  },
}

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T
const form = reactive<Settings>(clone(defaults))
const saving = reactive({ value: false })

watch(
  () => props.modelValue,
  (open) => {
    if (open) {
      Object.assign(form, clone(appSettings.value ?? defaults))
    }
  },
)

async function save(): Promise<void> {
  saving.value = true
  try {
    await saveSettings({
      terminal: form.terminal,
      behavior: form.behavior,
      sftp: form.sftp,
      monitor: form.monitor,
      security: form.security,
    })
    ElMessage.success('设置已保存')
    visible.value = false
  } catch (error) {
    ElMessage.error(errorMessage(error))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="设置" width="620px" append-to-body>
    <div class="sections">
      <section>
        <h4>终端</h4>
        <div class="grid">
          <label>字号</label>
          <el-input-number v-model="form.terminal.fontSize" :min="11" :max="18" />
          <label>行高</label>
          <el-input-number v-model="form.terminal.lineHeight" :min="1" :max="2" :step="0.05" :precision="2" />
          <label>回滚行数</label>
          <el-input-number v-model="form.terminal.scrollback" :min="1000" :max="100000" :step="1000" />
          <label>光标闪烁</label>
          <el-switch v-model="form.terminal.cursorBlink" />
          <label>字体</label>
          <el-input v-model="form.terminal.fontFamily" />
        </div>
      </section>

      <section>
        <h4>行为</h4>
        <div class="grid">
          <label>关闭标签确认</label>
          <el-switch v-model="form.behavior.confirmOnCloseTab" />
          <label>断线自动重连</label>
          <el-switch v-model="form.behavior.autoReconnect" />
          <label>启动恢复会话</label>
          <el-switch v-model="form.behavior.restoreSessions" />
          <label>最大重试</label>
          <el-input-number v-model="form.behavior.maxRetries" :min="0" :max="10" />
        </div>
      </section>

      <section>
        <h4>SFTP</h4>
        <div class="grid">
          <label>跟随终端目录</label>
          <el-switch v-model="form.sftp.followTerminal" />
          <label>显示隐藏文件</label>
          <el-switch v-model="form.sftp.showHidden" />
          <label>保留权限位</label>
          <el-switch v-model="form.sftp.preserveMode" />
          <label>冲突策略</label>
          <el-select v-model="form.sftp.conflictPolicy">
            <el-option label="询问" value="ask" />
            <el-option label="覆盖" value="overwrite" />
            <el-option label="跳过" value="skip" />
          </el-select>
          <label>默认下载目录</label>
          <el-input v-model="form.sftp.downloadDir" placeholder="留空使用 ~/Downloads/NestTerm" />
        </div>
      </section>

      <section>
        <h4>监控 / 安全</h4>
        <div class="grid">
          <label>刷新间隔 (ms)</label>
          <el-input-number v-model="form.monitor.intervalMs" :min="1000" :max="10000" :step="500" />
          <label>主机密钥策略</label>
          <el-select v-model="form.security.hostKeyPolicy">
            <el-option label="TOFU（首次信任）" value="tofu" />
            <el-option label="Strict（未知即拒绝）" value="strict" />
          </el-select>
        </div>
      </section>
    </div>
    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving.value" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.sections {
  max-height: 62vh;
  overflow-y: auto;
  padding-right: 6px;
}
section {
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid #1c2733;
}
section:last-child {
  border-bottom: none;
}
h4 {
  margin: 0 0 10px;
  color: #dbe6f3;
  font-size: 13px;
}
.grid {
  display: grid;
  grid-template-columns: 140px 1fr;
  align-items: center;
  gap: 8px 10px;
}
label {
  color: #8fa4b9;
  font-size: 12px;
}
</style>
