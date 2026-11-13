<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { monitorSample, type MonitorStats } from '../lib/monitor'
import { errorMessage } from '../lib/api'
import { appSettings } from '../stores/settings'

const props = defineProps<{ sessionId: string; status: string }>()

const stats = ref<MonitorStats | null>(null)
const error = ref('')
const sampling = ref(false)
const cpuHistory = ref<number[]>([])
const memHistory = ref<number[]>([])
let timer: number | null = null

const connected = computed(() => props.status === 'connected')

function formatBytes(value: number): string {
  if (!value) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let index = 0
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024
    index += 1
  }
  return `${size.toFixed(size >= 100 ? 0 : 1)} ${units[index]}`
}

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  return days > 0 ? `${days}天 ${hours}时` : `${hours}时 ${mins}分`
}

function percent(value: number): string {
  return `${Math.min(100, Math.max(0, value)).toFixed(1)}%`
}

async function sample(): Promise<void> {
  if (!connected.value || sampling.value) return
  sampling.value = true
  try {
    const next = await monitorSample(props.sessionId)
    stats.value = next
    error.value = ''
    cpuHistory.value = [...cpuHistory.value.slice(-29), next.cpuPercent]
    memHistory.value = [...memHistory.value.slice(-29), next.memPercent]
  } catch (caught) {
    error.value = errorMessage(caught)
    if (timer !== null) {
      window.clearInterval(timer)
      timer = null
    }
  } finally {
    sampling.value = false
  }
}

function restart(): void {
  error.value = ''
  stats.value = null
  cpuHistory.value = []
  memHistory.value = []
  if (timer !== null) window.clearInterval(timer)
  void sample()
  timer = window.setInterval(() => void sample(), appSettings.value?.monitor.intervalMs ?? 2000)
}

watch(
  () => appSettings.value?.monitor.intervalMs,
  () => restart(),
)

onMounted(() => restart())
onBeforeUnmount(() => {
  if (timer !== null) window.clearInterval(timer)
})
</script>

<template>
  <div class="monitor-pane">
    <div class="monitor-head">
      <div>
        <div class="title">系统监控</div>
        <div class="host">{{ stats?.host || '等待采样…' }}</div>
        <div class="os">{{ stats?.os || '' }}</div>
      </div>
      <button class="refresh" :disabled="sampling" title="立即刷新" @click="restart">↻</button>
    </div>

    <div v-if="error" class="monitor-error">
      <p>{{ error }}</p>
      <el-button size="small" @click="restart">重试</el-button>
    </div>

    <template v-else-if="stats">
      <div class="cards">
        <div class="card">
          <div class="card-head"><span>CPU</span><strong>{{ percent(stats.cpuPercent) }}</strong></div>
          <div class="bar"><i :style="{ width: percent(stats.cpuPercent) }" /></div>
          <div class="spark">
            <i v-for="(value, index) in cpuHistory" :key="index" :style="{ height: `${Math.max(4, value)}%` }" />
          </div>
        </div>
        <div class="card">
          <div class="card-head"><span>内存</span><strong>{{ percent(stats.memPercent) }}</strong></div>
          <div class="bar mem"><i :style="{ width: percent(stats.memPercent) }" /></div>
          <div class="meta">{{ formatBytes(stats.memUsed) }} / {{ formatBytes(stats.memTotal) }}</div>
          <div class="spark">
            <i v-for="(value, index) in memHistory" :key="index" :style="{ height: `${Math.max(4, value)}%`, background: '#4aa3ff' }" />
          </div>
        </div>
      </div>

      <div class="section">
        <div class="section-title">负载 / 运行时间</div>
        <div class="kv"><span>load 1m</span><b>{{ stats.load1.toFixed(2) }}</b></div>
        <div class="kv"><span>load 5m</span><b>{{ stats.load5.toFixed(2) }}</b></div>
        <div class="kv"><span>load 15m</span><b>{{ stats.load15.toFixed(2) }}</b></div>
        <div class="kv"><span>uptime</span><b>{{ formatUptime(stats.uptimeSecs) }}</b></div>
        <div class="kv"><span>swap</span><b>{{ formatBytes(stats.swapUsed) }} / {{ formatBytes(stats.swapTotal) }}</b></div>
      </div>

      <div class="section">
        <div class="section-title">磁盘 {{ stats.diskMount }}</div>
        <div class="bar disk"><i :style="{ width: percent(stats.diskPercent) }" /></div>
        <div class="meta">{{ formatBytes(stats.diskUsed) }} / {{ formatBytes(stats.diskTotal) }} · {{ percent(stats.diskPercent) }}</div>
      </div>

      <div class="section">
        <div class="section-title">网络（累计）</div>
        <div class="kv"><span>RX</span><b>{{ formatBytes(stats.netRxBytes) }}</b></div>
        <div class="kv"><span>TX</span><b>{{ formatBytes(stats.netTxBytes) }}</b></div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.monitor-pane {
  width: 280px;
  min-width: 260px;
  padding: 12px;
  overflow-y: auto;
  border-left: 1px solid #1b2530;
  background: #0b1118;
}
.monitor-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 12px;
}
.title {
  color: #dbe6f3;
  font-size: 13px;
  font-weight: 600;
}
.host {
  margin-top: 3px;
  color: #8fa4b9;
  font-family: 'JetBrains Mono', monospace;
  font-size: 11px;
  word-break: break-all;
}
.os {
  color: #5e7186;
  font-size: 10px;
}
.refresh {
  border: 1px solid #243241;
  border-radius: 4px;
  background: transparent;
  color: #9fb3c8;
  font-size: 15px;
  cursor: pointer;
}
.cards {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.card {
  padding: 10px;
  border: 1px solid #1c2733;
  border-radius: 6px;
  background: #0e1620;
}
.card-head {
  display: flex;
  justify-content: space-between;
  color: #a8b9cb;
  font-size: 12px;
}
.card-head strong {
  color: #e3eefb;
  font-family: 'JetBrains Mono', monospace;
  font-weight: 500;
}
.bar {
  height: 5px;
  margin-top: 8px;
  overflow: hidden;
  border-radius: 3px;
  background: #1a2632;
}
.bar i {
  display: block;
  height: 100%;
  border-radius: 3px;
  background: #35c46b;
  transition: width 0.35s ease;
}
.bar.mem i {
  background: #4aa3ff;
}
.bar.disk i {
  background: #e0a33a;
}
.spark {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 22px;
  margin-top: 8px;
}
.spark i {
  flex: 1;
  min-width: 2px;
  border-radius: 1px;
  background: rgba(53, 196, 107, 0.55);
}
.meta {
  margin-top: 6px;
  color: #6f8398;
  font-family: 'JetBrains Mono', monospace;
  font-size: 10px;
}
.section {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid #18222d;
}
.section-title {
  margin-bottom: 6px;
  color: #8fa4b9;
  font-size: 11px;
}
.kv {
  display: flex;
  justify-content: space-between;
  padding: 2px 0;
  color: #788da3;
  font-size: 11px;
}
.kv b {
  color: #c7d6e7;
  font-family: 'JetBrains Mono', monospace;
  font-weight: 500;
}
.monitor-error {
  padding: 12px;
  border: 1px solid #4a2b30;
  border-radius: 6px;
  color: #e39a9a;
  font-size: 12px;
}
.monitor-error p {
  margin: 0 0 8px;
  word-break: break-word;
}
</style>
