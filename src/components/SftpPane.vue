<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessage, ElMessageBox } from 'element-plus'
import { b64ToBytes, bytesToB64 } from '../lib/base64'
import { errorDetail, errorMessage } from '../lib/api'
import {
  sftpChmod,
  sftpDelete,
  sftpList,
  sftpMkdir,
  sftpOpen,
  sftpReadFile,
  sftpDragOut,
  sftpRename,
  sftpWriteFile,
} from '../lib/sftp'
import { appSettings } from '../stores/settings'
import { setHome } from '../stores/sessions'
import { sftpCancelTransfer, sftpDownload, sftpUpload } from '../lib/transfer'
import {
  activeTransferCount,
  pruneFinishedTransfers,
  transfersForSession,
  type TransferItem,
} from '../stores/transfers'
import type { SftpEntry, SftpListing, SshStatusState } from '../lib/types'

const props = defineProps<{
  sessionId: string
  status: SshStatusState
  cwd: string | null
  cwdSource: 'prompt' | 'manual' | null
}>()

const path = ref('')
const pathInput = ref('')
const listing = ref<SftpListing | null>(null)
const ready = ref(false)
const loading = ref(false)
const errorText = ref('')
const errorInfo = ref('')
const home = ref('/')
let navigationSeq = 0
let lastRequestedPath = ''
const follow = ref(appSettings.value?.sftp.followTerminal ?? true)
const showHidden = ref(appSettings.value?.sftp.showHidden ?? false)
const selected = ref<SftpEntry[]>([])
const paneRoot = ref<HTMLDivElement | null>(null)
const dragging = ref(false)
let unlistenDrag: (() => void) | null = null

const tray = computed<TransferItem[]>(() => transfersForSession(props.sessionId))
const runningTransfers = computed(() => activeTransferCount(props.sessionId))

const editorVisible = ref(false)
const editorPath = ref('')
const editorContent = ref('')
const editorMtime = ref<number | undefined>(undefined)
const editorSize = ref<number | undefined>(undefined)
const editorTruncated = ref(false)
const editorSaving = ref(false)

const entries = computed(() => listing.value?.entries ?? [])
const canUse = computed(() => props.status === 'connected')
const selectedOne = computed(() => (selected.value.length === 1 ? selected.value[0] : null))

let opening: Promise<void> | null = null

function apiCode(error: unknown): string | undefined {
  if (error && typeof error === 'object' && 'code' in error) {
    const code = (error as { code?: unknown }).code
    return typeof code === 'string' ? code : undefined
  }
  return undefined
}

function setError(error: unknown): void {
  errorText.value = errorMessage(error)
  errorInfo.value = errorDetail(error) ?? ''
  ElMessage.error(errorText.value)
}

function joinPath(dir: string, name: string): string {
  return dir === '/' ? `/${name}` : `${dir}/${name}`
}

function parentPath(value: string): string | null {
  if (value === '/') return null
  const index = value.lastIndexOf('/')
  return index <= 0 ? '/' : value.slice(0, index)
}

function formatSize(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`
  return `${(size / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function formatTime(ms: number): string {
  if (!ms) return '—'
  const date = new Date(ms)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function modeText(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(3, '0')
}

function kindIcon(kind: SftpEntry['kind']): string {
  if (kind === 'dir') return '📁'
  if (kind === 'symlink') return '🔗'
  return '📄'
}

async function navigate(target: string, silent = false): Promise<boolean> {
  const value = target.trim()
  const seq = ++navigationSeq
  lastRequestedPath = value
  if (!value) return false
  if (!value.startsWith('/')) {
    ElMessage.error('请输入绝对路径')
    return false
  }
  loading.value = true
  errorText.value = ''
  try {
    const result = await sftpList(props.sessionId, value, showHidden.value)
    // 首屏 home 等旧请求晚到：丢弃，避免把已切到的新目录覆盖回旧目录。
    if (seq !== navigationSeq) return true
    listing.value = result
    path.value = result.path
    pathInput.value = result.path
    selected.value = []
    return true
  } catch (error) {
    // 启发式 cd 的候选路径可能无效：静默忽略，不打断当前面板。
    if (!silent) setError(error)
    return false
  } finally {
    loading.value = false
  }
}

async function openSession(): Promise<void> {
  if (!canUse.value) return
  try {
    const opened = await sftpOpen(props.sessionId)
    home.value = opened.home
    setHome(props.sessionId, opened.home)
    ready.value = true
    // 首次导航成功后统一同步 home/current，避免打开时的旧 hint 覆盖 cd 的 current。
    // 已有关键 cwd 或已有导航发起时，不再无条件回 home。
    if (!lastRequestedPath) {
      await navigate(props.cwd || opened.home)
    }
  } catch (error) {
    setError(error)
  }
}

function ensureOpen(): Promise<void> {
  if (!opening) {
    opening = openSession().finally(() => {
      opening = null
    })
  }
  return opening
}

function refresh(): void {
  if (!ready.value) {
    void ensureOpen()
    return
  }
  if (path.value) void navigate(path.value)
}

function goUp(): void {
  // 用当前路径推导父目录：列表失败时 listing 可能还是旧目录，不能依赖它。
  const parent = parentPath(path.value)
  if (parent) void navigate(parent)
}

function goHome(): void {
  void navigate(home.value)
}

function onFollowChange(value: boolean | string | number): void {
  if (Boolean(value) && props.cwd) void navigate(props.cwd)
}

function onSelectionChange(rows: SftpEntry[]): void {
  selected.value = rows
}

function percent(item: TransferItem): number {
  if (!item.totalBytes) return item.status === 'completed' ? 100 : 0
  return Math.min(100, Math.round((item.bytes / item.totalBytes) * 100))
}

function speedText(bps: number): string {
  if (bps < 1024) return `${bps} B/s`
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`
  return `${(bps / 1024 / 1024).toFixed(1)} MB/s`
}

function transferStatus(status: TransferItem['status']): string {
  if (status === 'completed') return '完成'
  if (status === 'cancelled') return '已取消'
  if (status === 'failed') return '失败'
  return '进行中'
}

function trayName(item: TransferItem): string {
  return item.direction === 'upload' ? item.remotePath.split('/').pop() ?? item.remotePath : item.remotePath.split('/').pop() ?? item.remotePath
}

async function cancelTransfer(item: TransferItem): Promise<void> {
  try {
    await sftpCancelTransfer(item.transferId)
  } catch (error) {
    setError(error)
  }
}

function clearFinished(): void {
  pruneFinishedTransfers(props.sessionId)
}

async function uploadPaths(paths: string[]): Promise<void> {
  if (!canUse.value || paths.length === 0) return
  try {
    const result = await sftpUpload({
      sessionId: props.sessionId,
      localPaths: paths,
      remoteDir: path.value,
      conflictPolicy: appSettings.value?.sftp.conflictPolicy ?? 'ask',
      preserveMode: appSettings.value?.sftp.preserveMode ?? false,
    })
    if (result.skipped.length > 0) ElMessage.info(`已跳过 ${result.skipped.length} 个同名文件`)
    await navigate(path.value)
  } catch (error) {
    if (apiCode(error) !== 'SFTP_CONFLICT') {
      setError(error)
      return
    }
    let policy: 'overwrite' | 'skip' = 'skip'
    try {
      await ElMessageBox.confirm('远端已存在同名文件，覆盖？选择取消则跳过。', '文件冲突', {
        confirmButtonText: '覆盖',
        cancelButtonText: '跳过',
        type: 'warning',
      })
      policy = 'overwrite'
    } catch {
      policy = 'skip'
    }
    try {
      const result = await sftpUpload({
        sessionId: props.sessionId,
        localPaths: paths,
        remoteDir: path.value,
        conflictPolicy: policy,
        preserveMode: appSettings.value?.sftp.preserveMode ?? false,
      })
      if (result.skipped.length > 0) ElMessage.info(`已跳过 ${result.skipped.length} 个同名文件`)
      await navigate(path.value)
    } catch (retryError) {
      setError(retryError)
    }
  }
}

async function uploadPrompt(): Promise<void> {
  try {
    const result = await ElMessageBox.prompt('本地文件绝对路径（多个文件可直接拖拽到面板）', '上传文件', {
      confirmButtonText: '上传',
      cancelButtonText: '取消',
      inputPlaceholder: '/Users/you/file.txt',
    })
    await uploadPaths([result.value.trim()])
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') setError(error)
  }
}

async function dragOut(entry: SftpEntry): Promise<void> {
  try {
    await sftpDragOut(props.sessionId, entry.path)
  } catch (error) {
    ElMessage.error(`拖拽下载失败：${errorMessage(error)}`)
  }
}

async function downloadSelected(): Promise<void> {
  const paths = selected.value.map((entry) => entry.path)
  if (paths.length === 0) return
  let localDir: string | undefined
  try {
    const chosen = await open({
      directory: true,
      multiple: false,
      title: '选择下载目录',
      defaultPath: appSettings.value?.sftp.downloadDir || undefined,
    })
    if (!chosen || typeof chosen !== 'string') return
    localDir = chosen
  } catch (error) {
    ElMessage.error(`打开目录选择器失败：${errorMessage(error)}`)
    return
  }

  try {
    const result = await sftpDownload({
      sessionId: props.sessionId,
      remotePaths: paths,
      localDir,
      conflictPolicy: appSettings.value?.sftp.conflictPolicy ?? 'ask',
      preserveMode: appSettings.value?.sftp.preserveMode ?? false,
    })
    if (result.localDir) ElMessage.success(`已下载到 ${result.localDir}`)
  } catch (error) {
    if (apiCode(error) !== 'SFTP_CONFLICT') {
      setError(error)
      return
    }
    let policy: 'overwrite' | 'skip' = 'skip'
    try {
      await ElMessageBox.confirm('本地已存在同名文件，覆盖？选择取消则跳过。', '文件冲突', {
        confirmButtonText: '覆盖',
        cancelButtonText: '跳过',
        type: 'warning',
      })
      policy = 'overwrite'
    } catch {
      policy = 'skip'
    }
    try {
      const result = await sftpDownload({
        sessionId: props.sessionId,
        remotePaths: paths,
        localDir,
        conflictPolicy: policy,
        preserveMode: appSettings.value?.sftp.preserveMode ?? false,
      })
      if (result.localDir) ElMessage.success(`已下载到 ${result.localDir}`)
    } catch (retryError) {
      setError(retryError)
    }
  }
}

function insidePane(position: { x: number; y: number }): boolean {
  const element = paneRoot.value
  if (!element) return false
  const rect = element.getBoundingClientRect()
  const scale = window.devicePixelRatio || 1
  const x = position.x / scale
  const y = position.y / scale
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

function onRowDblClick(row: SftpEntry): void {
  if (row.kind === 'dir' || row.kind === 'symlink') {
    void navigate(row.path)
  } else {
    void openEditor(row)
  }
}

function editSelected(): void {
  if (selectedOne.value) void openEditor(selectedOne.value)
}

async function openEditor(entry: SftpEntry): Promise<void> {
  try {
    const result = await sftpReadFile(props.sessionId, entry.path)
    if (result.isBinary) {
      ElMessage.warning('二进制文件不支持在线编辑，请先下载')
      return
    }
    editorPath.value = result.path
    editorContent.value = new TextDecoder().decode(b64ToBytes(result.content))
    editorMtime.value = result.mtimeMs
    editorSize.value = result.size
    editorTruncated.value = result.truncated
    editorVisible.value = true
    if (result.truncated) {
      ElMessage.warning('文件超过 2MB，仅加载前 2MB，保存已禁用')
    }
  } catch (error) {
    setError(error)
  }
}

async function saveEditor(): Promise<void> {
  if (editorTruncated.value) return
  editorSaving.value = true
  try {
    await sftpWriteFile({
      sessionId: props.sessionId,
      path: editorPath.value,
      content: bytesToB64(new TextEncoder().encode(editorContent.value)),
      expectedMtimeMs: editorMtime.value,
      expectedSize: editorSize.value,
    })
    editorVisible.value = false
    ElMessage.success('已保存')
    await navigate(path.value)
  } catch (error) {
    if (apiCode(error) === 'SFTP_CONFLICT') {
      ElMessage.error('远端文件已被修改，请关闭后重新打开再保存')
    } else {
      setError(error)
    }
  } finally {
    editorSaving.value = false
  }
}

async function mkdir(): Promise<void> {
  try {
    const result = await ElMessageBox.prompt('目录名称', '新建目录', {
      confirmButtonText: '创建',
      cancelButtonText: '取消',
      inputValidator: (value) => {
        const name = value?.trim()
        if (!name) return '请输入名称'
        if (name.includes('/')) return '名称不能包含 /'
        return true
      },
    })
    await sftpMkdir(props.sessionId, joinPath(path.value, result.value.trim()))
    await navigate(path.value)
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') setError(error)
  }
}

async function rename(): Promise<void> {
  const entry = selectedOne.value
  if (!entry) return
  try {
    const result = await ElMessageBox.prompt('新名称', '重命名', {
      confirmButtonText: '重命名',
      cancelButtonText: '取消',
      inputValue: entry.name,
      inputValidator: (value) => {
        const name = value?.trim()
        if (!name) return '请输入名称'
        if (name.includes('/')) return '名称不能包含 /'
        return true
      },
    })
    const parent = parentPath(entry.path) ?? '/'
    await sftpRename(props.sessionId, entry.path, joinPath(parent, result.value.trim()))
    await navigate(path.value)
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') setError(error)
  }
}

async function chmod(): Promise<void> {
  const entry = selectedOne.value
  if (!entry) return
  try {
    const result = await ElMessageBox.prompt('八进制权限，例如 644 / 755', '修改权限', {
      confirmButtonText: '应用',
      cancelButtonText: '取消',
      inputValue: modeText(entry.mode),
      inputValidator: (value) => (/^[0-7]{1,4}$/.test(value?.trim() ?? '') ? true : '请输入 0–07777 的八进制数'),
    })
    await sftpChmod(props.sessionId, entry.path, Number.parseInt(result.value.trim(), 8))
    await navigate(path.value)
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') setError(error)
  }
}

async function remove(): Promise<void> {
  if (selected.value.length === 0) return
  const count = selected.value.length
  try {
    await ElMessageBox.confirm(`删除选中的 ${count} 项？目录会递归删除，且不可恢复。`, '删除', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消',
    })
    await sftpDelete(
      props.sessionId,
      selected.value.map((entry) => entry.path),
      true,
    )
    selected.value = []
    await navigate(path.value)
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') setError(error)
  }
}

watch(
  () => appSettings.value?.sftp,
  (settings) => {
    if (!settings) return
    follow.value = settings.followTerminal
    showHidden.value = settings.showHidden
    if (ready.value) void refresh()
  },
)

watch(
  () => props.status,
  (status) => {
    if (status === 'connected') {
      void ensureOpen()
    } else {
      ready.value = false
      listing.value = null
      lastRequestedPath = ''
    }
  },
)

watch(
  () => props.cwd,
  async (cwd) => {
    if (!follow.value || !cwd || !ready.value || cwd === path.value) return
    const ok = await navigate(cwd)
    if (!ok) {
      await new Promise((resolve) => window.setTimeout(resolve, 200))
      if (props.cwd === cwd && cwd !== path.value) await navigate(cwd)
    }
  },
)

onMounted(async () => {
  if (canUse.value) void ensureOpen()
  try {
    unlistenDrag = await getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload
      if (payload.type === 'enter' || payload.type === 'over') {
        dragging.value = insidePane(payload.position)
      } else if (payload.type === 'leave') {
        dragging.value = false
      } else if (payload.type === 'drop' && 'paths' in payload) {
        const inside = insidePane(payload.position)
        dragging.value = false
        if (inside && payload.paths.length > 0) void uploadPaths(payload.paths)
      }
    })
  } catch (error) {
    console.warn('[nestterm] 注册拖拽事件失败：', error)
  }
})

onBeforeUnmount(() => {
  unlistenDrag?.()
  unlistenDrag = null
})
</script>

<template>
  <div ref="paneRoot" class="sftp-pane" :class="{ dragging }">
    <div class="sftp-head">
      <span class="sftp-title">文件</span>
      <span class="sftp-sub">{{ path || '连接后加载' }}</span>
      <span class="spacer" />
      <el-checkbox v-model="follow" size="small" @change="onFollowChange">跟随终端</el-checkbox>
      <el-checkbox v-model="showHidden" size="small" @change="() => refresh()">隐藏文件</el-checkbox>
    </div>

    <div class="sftp-pathbar">
      <button class="icon-btn" title="上级" :disabled="!listing?.parent" @click="goUp">↑</button>
      <button class="icon-btn" title="Home" @click="goHome">⌂</button>
      <button class="icon-btn" title="刷新" @click="refresh">⟳</button>
      <el-input
        v-model="pathInput"
        size="small"
        spellcheck="false"
        placeholder="/path/to/dir"
        @keyup.enter="navigate(pathInput)"
      />
    </div>

    <div class="sftp-toolbar">
      <el-button size="small" :disabled="!canUse" @click="mkdir">新建目录</el-button>
      <el-button size="small" :disabled="!canUse" @click="uploadPrompt">上传</el-button>
      <el-button size="small" :disabled="!selectedOne" @click="rename">重命名</el-button>
      <el-button size="small" :disabled="!selectedOne" @click="chmod">权限</el-button>
      <el-button size="small" :disabled="selected.length === 0" @click="downloadSelected">下载</el-button>
      <el-button size="small" type="danger" plain :disabled="selected.length === 0" @click="remove">
        删除
      </el-button>
      <el-button size="small" :disabled="!selectedOne || selectedOne.kind === 'dir'" @click="editSelected">
        编辑
      </el-button>
    </div>

    <el-alert
      v-if="errorText"
      class="sftp-error"
      type="error"
      :closable="true"
      show-icon
      :title="errorText"
      @close="errorText = ''"
    >
      <template #default>
        <p v-if="errorInfo" class="sftp-error-detail">{{ errorInfo }}</p>
      </template>
    </el-alert>

    <div class="sftp-table-wrap">
      <el-table
        v-loading="loading"
        :data="entries"
        size="small"
        height="100%"
        empty-text="空目录"
        @row-dblclick="onRowDblClick"
        @selection-change="onSelectionChange"
      >
        <el-table-column type="selection" width="34" />
        <el-table-column label="名称" min-width="170">
          <template #default="{ row }">
            <span class="kind">{{ kindIcon(row.kind) }}</span>
            <span
              class="drag-handle"
              title="按住拖到 Finder"
              @mousedown.stop.prevent="dragOut(row as SftpEntry)"
            >⠿</span>
            <span class="name">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="82" align="right">
          <template #default="{ row }">{{ row.kind === 'dir' ? '—' : formatSize(row.size) }}</template>
        </el-table-column>
        <el-table-column label="修改时间" width="128">
          <template #default="{ row }">{{ formatTime(row.mtimeMs) }}</template>
        </el-table-column>
        <el-table-column label="权限" width="56">
          <template #default="{ row }">{{ modeText(row.mode) }}</template>
        </el-table-column>
      </el-table>
    </div>

    <div class="sftp-foot">
      <span>{{ entries.length }} 项</span>
      <span v-if="listing?.truncated" class="warn">仅显示前 5000 项</span>
      <span class="spacer" />
      <span class="hint">拖拽文件/目录上传 · 按住 ⠿ 拖到 Finder 下载</span>
    </div>

    <div v-if="tray.length > 0" class="transfer-tray">
      <div class="tray-head">
        <span>传输</span>
        <span v-if="runningTransfers > 0" class="running">{{ runningTransfers }} 进行中</span>
        <span class="spacer" />
        <button class="link" @click="clearFinished">清理已完成</button>
      </div>
      <div v-for="item in tray" :key="item.transferId" class="transfer-item">
        <span class="dir">{{ item.direction === 'upload' ? '↑' : '↓' }}</span>
        <span class="tname" :title="item.remotePath">{{ trayName(item) }}</span>
        <el-progress
          class="tprogress"
          :percentage="percent(item)"
          :stroke-width="4"
          :show-text="false"
          :status="item.status === 'failed' ? 'exception' : undefined"
        />
        <span class="tmeta">{{ percent(item) }}% · {{ speedText(item.speedBps) }}</span>
        <button v-if="item.status === 'running'" class="link" @click="cancelTransfer(item)">取消</button>
        <span v-else class="tstatus" :class="item.status">{{ transferStatus(item.status) }}</span>
      </div>
    </div>

    <div v-if="dragging" class="drop-overlay">松开上传到 {{ path }}</div>

    <el-dialog v-model="editorVisible" :title="editorPath" width="760px" top="6vh" destroy-on-close>
      <div class="editor-meta">
        <span>{{ editorSize }} 字节</span>
        <span v-if="editorTruncated" class="warn">超过 2MB，仅显示前 2MB，保存已禁用</span>
      </div>
      <el-input v-model="editorContent" type="textarea" :rows="20" class="editor" spellcheck="false" />
      <template #footer>
        <el-button @click="editorVisible = false">取消</el-button>
        <el-button type="primary" :loading="editorSaving" :disabled="editorTruncated" @click="saveEditor">
          保存
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.sftp-pane {
  position: relative;
  display: flex;
  flex: 0 0 430px;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  border-left: 1px solid #18212b;
  background: #090e13;
}

.sftp-head {
  display: flex;
  flex: 0 0 34px;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  border-bottom: 1px solid #18212b;
  font-size: 12px;
}

.sftp-title {
  font-weight: 600;
}

.sftp-sub {
  overflow: hidden;
  max-width: 180px;
  color: var(--el-text-color-placeholder);
  font-family: 'SF Mono', Menlo, monospace;
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sftp-pathbar {
  display: flex;
  flex: 0 0 36px;
  align-items: center;
  gap: 4px;
  padding: 0 8px;
  border-bottom: 1px solid #141c25;
}

.icon-btn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--el-text-color-secondary);
  cursor: pointer;
}

.icon-btn:hover:not(:disabled) {
  background: #1a2530;
  color: #3ddc97;
}

.icon-btn:disabled {
  opacity: 0.35;
  cursor: default;
}

.sftp-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 6px 8px;
  border-bottom: 1px solid #141c25;
}

.sftp-toolbar :deep(.el-button + .el-button) {
  margin-left: 0;
}

.sftp-error {
  margin: 6px 8px 0;
}

.sftp-error-detail {
  margin: 4px 0 0;
  font-family: 'SF Mono', Menlo, monospace;
  font-size: 10px;
  white-space: pre-wrap;
}

.sftp-table-wrap {
  flex: 1;
  min-height: 0;
  padding: 4px 6px;
}

.sftp-table-wrap :deep(.el-table) {
  --el-table-bg-color: transparent;
  --el-table-tr-bg-color: transparent;
  --el-table-header-bg-color: #0d1319;
  background: transparent;
  font-size: 11.5px;
}

.kind {
  display: inline-block;
  width: 16px;
  margin-right: 4px;
  font-size: 11px;
}

.name {
  font-family: 'SF Mono', Menlo, monospace;
  font-size: 11.5px;
}

.sftp-foot {
  display: flex;
  flex: 0 0 24px;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  border-top: 1px solid #141c25;
  color: var(--el-text-color-placeholder);
  font-size: 10px;
}

.warn {
  color: #e6c07b;
}

.hint {
  opacity: 0.7;
}

.editor-meta {
  display: flex;
  gap: 12px;
  margin-bottom: 8px;
  color: var(--el-text-color-secondary);
  font-size: 11px;
}

.transfer-tray {
  flex: 0 0 auto;
  max-height: 150px;
  overflow-y: auto;
  padding: 4px 8px 6px;
  border-top: 1px solid #18212b;
  background: #080d12;
}

.tray-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 0 4px;
  color: var(--el-text-color-secondary);
  font-size: 10.5px;
}

.running {
  color: #3ddc97;
}

.link {
  border: none;
  background: transparent;
  color: var(--el-text-color-secondary);
  font-size: 10.5px;
  cursor: pointer;
}

.link:hover {
  color: #3ddc97;
}

.transfer-item {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  font-size: 10.5px;
}

.dir {
  width: 10px;
  color: #6ea8fe;
}

.tname {
  overflow: hidden;
  flex: 0 0 96px;
  color: var(--el-text-color-regular);
  font-family: 'SF Mono', Menlo, monospace;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tprogress {
  flex: 1;
  min-width: 40px;
}

.tmeta {
  flex: 0 0 96px;
  color: var(--el-text-color-placeholder);
  font-size: 10px;
  text-align: right;
}

.tstatus.completed {
  color: #3ddc97;
}

.tstatus.failed {
  color: #f26d78;
}

.tstatus.cancelled {
  color: #e6c07b;
}

.drop-overlay {
  position: absolute;
  inset: 6px;
  z-index: 20;
  display: grid;
  place-items: center;
  border: 2px dashed #3ddc97;
  border-radius: 8px;
  background: rgba(9, 14, 19, 0.88);
  color: #3ddc97;
  font-size: 12px;
  pointer-events: none;
}

.editor :deep(textarea) {
  font-family: 'SF Mono', Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.55;
}

.drag-handle {
  display: inline-block;
  margin-right: 4px;
  padding: 0 2px;
  color: #4d6378;
  cursor: grab;
  user-select: none;
}
.drag-handle:hover {
  color: #3ddc97;
}
</style>
