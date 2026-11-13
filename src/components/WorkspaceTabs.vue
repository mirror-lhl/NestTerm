<script setup lang="ts">
import { computed, ref } from 'vue'
import { ElMessageBox } from 'element-plus'
import TerminalPane from './TerminalPane.vue'
import SftpPane from './SftpPane.vue'
import ForwardDialog from './ForwardDialog.vue'
import MonitorPane from './MonitorPane.vue'
import { errorMessage } from '../lib/api'
import { sshDisconnect } from '../lib/ssh'
import { activeId, activateTab, activeTab, removeTab, tabs } from '../stores/sessions'
import { appSettings } from '../stores/settings'

const props = defineProps<{ initialMonitor?: boolean }>()
const emit = defineEmits<{ new: [] }>()

const sftpVisible = ref(true)
const forwardVisible = ref(false)
const monitorVisible = ref(Boolean(props.initialMonitor))
if (monitorVisible.value) sftpVisible.value = false
const canUseSftp = computed(() => activeTab.value?.status === 'connected')

async function closeTab(sessionId: string, title: string, status: string): Promise<void> {
  const confirmOnClose = appSettings.value?.behavior.confirmOnCloseTab ?? true
  if (
    confirmOnClose &&
    (status === 'connected' || status === 'connecting' || status === 'reconnecting')
  ) {
    try {
      await ElMessageBox.confirm(`关闭「${title}」会断开当前连接，继续？`, '关闭标签', {
        type: 'warning',
        confirmButtonText: '关闭',
        cancelButtonText: '取消',
      })
    } catch {
      return
    }
  }
  try {
    await sshDisconnect(sessionId)
  } catch (error) {
    console.warn('[nestterm] 断开会话失败：', errorMessage(error))
  }
  removeTab(sessionId)
}
</script>

<template>
  <div class="workspace-tabs">
    <div class="tabbar">
      <div
        v-for="tab in tabs"
        :key="tab.sessionId"
        class="tab"
        :class="{ active: tab.sessionId === activeId }"
        :title="tab.message || tab.title"
        @click="activateTab(tab.sessionId)"
      >
        <span class="tab-dot" :class="`dot-${tab.status}`" />
        <span class="tab-title">{{ tab.title }}</span>
        <span class="tab-close" @click.stop="closeTab(tab.sessionId, tab.title, tab.status)">×</span>
      </div>
      <button class="tab-new" title="新建连接" @click="emit('new')">＋</button>
      <span class="spacer" />
      <button
        class="dock-toggle"
        :class="{ on: sftpVisible && canUseSftp }"
        :disabled="!canUseSftp"
        :title="canUseSftp ? '显示/隐藏文件面板' : '连接后可用'"
        @click="sftpVisible = !sftpVisible; if (sftpVisible) monitorVisible = false"
      >
        文件
      </button>
      <button
        class="dock-toggle"
        :class="{ on: monitorVisible && canUseSftp }"
        :disabled="!canUseSftp"
        :title="canUseSftp ? '显示/隐藏系统监控' : '连接后可用'"
        @click="monitorVisible = !monitorVisible; if (monitorVisible) sftpVisible = false"
      >
        监控
      </button>
      <button
        class="dock-toggle"
        :class="{ on: forwardVisible && canUseSftp }"
        :disabled="!canUseSftp || !activeTab?.profileId"
        :title="activeTab?.profileId ? '端口转发（-L / SOCKS5）' : '保存连接后可配置转发'"
        @click="forwardVisible = !forwardVisible"
      >
        转发
      </button>
    </div>

    <div class="workspace-main">
      <div class="panes">
        <TerminalPane
          v-for="tab in tabs"
          v-show="tab.sessionId === activeId"
          :key="tab.sessionId"
          :session-id="tab.sessionId"
          :status="tab.status"
          :active="tab.sessionId === activeId"
        />
      </div>

      <SftpPane
        v-if="activeTab"
        v-show="sftpVisible && canUseSftp"
        :key="activeTab.sessionId"
        :session-id="activeTab.sessionId"
        :status="activeTab.status"
        :cwd="activeTab.cwd"
        :cwd-source="activeTab.cwdSource"
      />
      <MonitorPane
        v-if="activeTab && monitorVisible && canUseSftp"
        :key="`monitor-${activeTab.sessionId}`"
        :session-id="activeTab.sessionId"
        :status="activeTab.status"
      />
      <ForwardDialog
        v-if="activeTab"
        v-model="forwardVisible"
        :session-id="activeTab.sessionId"
        :profile-id="activeTab.profileId"
      />
    </div>
  </div>
</template>

<style scoped>
.workspace-tabs {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.tabbar {
  display: flex;
  flex: 0 0 36px;
  align-items: stretch;
  gap: 2px;
  padding: 0 6px;
  overflow-x: auto;
  border-bottom: 1px solid #18212b;
  background: #080d12;
}

.tabbar::-webkit-scrollbar {
  height: 0;
}

.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 220px;
  padding: 0 8px 0 10px;
  border-bottom: 2px solid transparent;
  color: var(--el-text-color-secondary);
  font-size: 12px;
  font-family: 'SF Mono', Menlo, monospace;
  white-space: nowrap;
  user-select: none;
  cursor: default;
}

.tab:hover {
  background: #10161d;
}

.tab.active {
  border-bottom-color: #3ddc97;
  background: #0d1319;
  color: var(--el-text-color-primary);
}

.tab-dot {
  width: 6px;
  height: 6px;
  flex: 0 0 6px;
  border-radius: 50%;
  background: #3a4654;
}

.dot-connected {
  background: #3ddc97;
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

.tab-title {
  overflow: hidden;
  text-overflow: ellipsis;
}

.tab-close {
  padding: 0 2px;
  border-radius: 3px;
  font-size: 13px;
  line-height: 1;
  opacity: 0.55;
}

.tab-close:hover {
  background: #24303c;
  opacity: 1;
}

.tab-new {
  align-self: center;
  padding: 2px 8px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--el-text-color-secondary);
  font-size: 15px;
  cursor: pointer;
}

.tab-new:hover {
  background: #141c25;
  color: #3ddc97;
}

.workspace-main {
  display: flex;
  flex: 1;
  min-height: 0;
}

.panes {
  position: relative;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.dock-toggle {
  align-self: center;
  margin-right: 6px;
  padding: 2px 10px;
  border: 1px solid #26313d;
  border-radius: 5px;
  background: transparent;
  color: var(--el-text-color-secondary);
  font-size: 11px;
  cursor: pointer;
}

.dock-toggle:hover:not(:disabled) {
  border-color: #3ddc97;
  color: #3ddc97;
}

.dock-toggle.on {
  border-color: #2f6b53;
  background: #12251d;
  color: #3ddc97;
}

.dock-toggle:disabled {
  opacity: 0.35;
  cursor: default;
}

.panes > :deep(.terminal-host) {
  position: absolute;
  inset: 0;
}
</style>
