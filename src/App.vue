<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import SessionTree from './components/SessionTree.vue'
import WorkspaceTabs from './components/WorkspaceTabs.vue'
import ProfileDialog from './components/ProfileDialog.vue'
import SettingsDialog from './components/SettingsDialog.vue'
import KnownHostsDialog from './components/KnownHostsDialog.vue'
import {
  appInfo,
  errorDetail,
  errorMessage,
  groupCreate,
  groupDelete,
  groupUpdate,
  profileDelete,
  profileDuplicate,
} from './lib/api'
import {
  onSshData,
  onSshHostKey,
  onSshStatus,
  sshConnect,
  sshDisconnect,
  sshWrite,
} from './lib/ssh'
import type { AppInfo, AuthInput, ConnProfile, Group } from './lib/types'
import { finalshellImport, finalshellPreview } from './lib/import'
import { nesttermExport, nesttermImport } from './lib/export'
import {
  forwardRuleCreate,
  forwardRulesList,
  forwardRunning,
  forwardStart,
} from './lib/forward'
import { b64ToBytes, bytesToB64 } from './lib/base64'
import { profileState, refreshProfiles } from './stores/profiles'
import { appSettings, loadSettings } from './stores/settings'
import { sessionsSave, sessionsSaved } from './lib/session_state'
import { finishTransfer, transferItems, upsertTransfer } from './stores/transfers'
import {
  onTransferDone,
  onTransferProgress,
  sftpCancelTransfer,
  sftpDownload,
  sftpUpload,
} from './lib/transfer'
import {
  sftpChmod,
  sftpDelete,
  sftpList,
  sftpMkdir,
  sftpOpen,
  sftpReadFile,
  sftpRename,
  sftpStat,
  sftpWriteFile,
} from './lib/sftp'
import {
  activateTab,
  activeTab,
  addTab,
  findTabByProfile,
  getTerminalSize,
  removeTab,
  setFingerprint,
  tabs,
  updateStatus,
  updateTabTitle,
  writeTerminal,
} from './stores/sessions'

const info = ref<AppInfo | null>(null)
const dialogVisible = ref(false)
const dialogProfile = ref<ConnProfile | null>(null)
const dialogMode = ref<'create' | 'edit' | 'connect'>('create')
const presetGroupId = ref<string | null>(null)
const treeRef = ref<InstanceType<typeof SessionTree> | null>(null)
const settingsVisible = ref(false)
const knownHostsVisible = ref(false)

const unlisteners: Array<() => void> = []
/** 仅 debug 联调：自动连接后发送的初始化命令（发一次即清空）。 */
let pendingDevCommand: string | null = null
/** 同一错误可能同时来自事件与 invoke 拒绝，做 3 秒去重，避免双 toast。 */
const reportedErrors = new Set<string>()

function reportError(sessionId: string, message: string): void {
  const key = `${sessionId}:${message}`
  if (reportedErrors.has(key)) return
  reportedErrors.add(key)
  window.setTimeout(() => reportedErrors.delete(key), 3000)
  ElMessage.error(message)
}

let saveSessionsTimer = 0

function saveOpenProfiles(): void {
  const ids = Array.from(
    new Set(tabs.value.map((tab) => tab.profileId).filter((id): id is string => Boolean(id))),
  )
  window.clearTimeout(saveSessionsTimer)
  saveSessionsTimer = window.setTimeout(() => {
    void sessionsSave(ids).catch(() => undefined)
  }, 300)
}

async function restoreOpenProfiles(): Promise<void> {
  let ids: string[] = []
  try {
    ids = await sessionsSaved()
  } catch {
    return
  }
  if (!Array.isArray(ids) || ids.length === 0) return
  for (const id of ids.slice(0, 8)) {
    const profile = profileState.profiles.find((item) => item.id === id)
    if (!profile) continue
    if (profile.authType === 'password' && !profile.hasPassword) continue
    if (tabs.value.some((tab) => tab.profileId === id)) continue
    await connectProfile(profile)
  }
}

const startupError = computed(() => info.value?.startupError ?? null)
const startupDetail = computed(() => (startupError.value ? errorDetail(startupError.value) : undefined))

const statusLabel = computed(() => {
  const tab = activeTab.value
  if (!tab) return '未连接'
  switch (tab.status) {
    case 'connecting':
      return '连接中'
    case 'connected':
      return '已连接'
    case 'reconnecting':
      return '重连中'
    case 'error':
      return '错误'
    case 'disconnected':
      return '已断开'
    default:
      return '未连接'
  }
})

const statusTagType = computed<'success' | 'warning' | 'danger' | 'info'>(() => {
  const tab = activeTab.value
  if (!tab) return 'info'
  if (tab.status === 'connected') return 'success'
  if (tab.status === 'connecting' || tab.status === 'reconnecting') return 'warning'
  if (tab.status === 'error') return 'danger'
  return 'info'
})

onMounted(async () => {
  try {
    info.value = await appInfo()
  } catch (error) {
    ElMessage.error(`读取应用信息失败：${errorMessage(error)}`)
  }

  await Promise.all([refreshProfiles(), loadSettings()])

  unlisteners.push(
    await onSshData((payload) => {
      writeTerminal(payload.sessionId, payload.data)
    }),
  )

  unlisteners.push(
    await onSshStatus((payload) => {
      updateStatus(payload)
      if (payload.state === 'error') {
        reportError(payload.sessionId, payload.message || '连接异常')
      }
      if (payload.state === 'connected') {
        void refreshProfiles()
      }
    }),
  )

  unlisteners.push(
    await onTransferProgress((payload) => {
      upsertTransfer(payload)
    }),
  )

  unlisteners.push(
    await onTransferDone((payload) => {
      finishTransfer(payload)
    }),
  )

  unlisteners.push(
  )

  unlisteners.push(
    await onSshHostKey((payload) => {
      setFingerprint(payload.sessionId, payload.fingerprint)
      if (payload.learned) {
        ElMessage.warning(`首次连接该主机，已记录指纹：${payload.fingerprint}`)
      }
    }),
  )

  window.addEventListener('keydown', onKeydown)
  if (info.value?.dev?.importTest) {
    await runImportTest()
  }
  if (info.value?.dev?.exportTest) {
    await runExportTest()
  }
  if (info.value?.dev?.jsonImportPath) {
    await runJsonImportAuto(info.value.dev.jsonImportPath)
  }
  const autoConnected = Boolean(info.value?.dev?.autoConnect)
  await maybeAutoConnect()
  if (!autoConnected && (appSettings.value?.behavior.restoreSessions ?? true)) {
    await restoreOpenProfiles()
  }
  if (info.value?.dev?.settingsPanel) settingsVisible.value = true
})

async function runSftpSelftest(sessionId: string): Promise<void> {
  const steps: string[] = []
  let ok = true
  let home = '/tmp'
  try {
    const opened = await sftpOpen(sessionId)
    home = opened.home
    const dir = `${home}/nestterm-probe-${Date.now()}`
    steps.push(`mkdir ${dir}`)
    await sftpMkdir(sessionId, dir)

    const payload = 'hello nestterm'
    const file = `${dir}/probe.txt`
    steps.push('write')
    await sftpWriteFile({
      sessionId,
      path: file,
      content: bytesToB64(new TextEncoder().encode(payload)),
    })

    steps.push('read')
    const read = await sftpReadFile(sessionId, file)
    const text = new TextDecoder().decode(b64ToBytes(read.content))
    if (text !== payload) throw new Error(`read mismatch: ${text}`)

    steps.push('chmod 600')
    await sftpChmod(sessionId, file, 0o600)
    steps.push('stat')
    const stat = await sftpStat(sessionId, file)
    if ((stat.mode & 0o777) !== 0o600) {
      throw new Error(`mode unexpected: ${stat.mode.toString(8)}`)
    }

    steps.push('rename')
    await sftpRename(sessionId, file, `${dir}/probe2.txt`)
    steps.push('list')
    const listing = await sftpList(sessionId, dir)
    if (!listing.entries.some((entry) => entry.name === 'probe2.txt')) {
      throw new Error('rename not reflected in listing')
    }

    steps.push('delete recursive')
    await sftpDelete(sessionId, [dir], true)
  } catch (error) {
    ok = false
    steps.push(`FAIL: ${errorMessage(error)}`)
  }

  try {
    await sftpWriteFile({
      sessionId,
      path: `${home}/nestterm-selftest.json`,
      content: bytesToB64(
        new TextEncoder().encode(JSON.stringify({ ok, steps, at: new Date().toISOString() }, null, 2)),
      ),
    })
  } catch (error) {
    console.error('[nestterm] 写入自检报告失败：', error)
  }

  if (ok) {
    ElMessage.success('SFTP 自检通过：write/read/chmod/rename/list/delete')
  } else {
    ElMessage.error('SFTP 自检失败，详见 ~/nestterm-selftest.json')
  }
}

function devErrorCode(error: unknown): string | undefined {
  if (error && typeof error === 'object' && 'code' in error) {
    return String((error as { code?: unknown }).code)
  }
  return undefined
}

async function waitTransfers(ids: string[], timeoutMs = 90_000): Promise<{ ok: boolean; errors: string[] }> {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const items = transferItems.value.filter((item) => ids.includes(item.transferId))
    if (items.length === ids.length && items.every((item) => item.status !== 'running')) {
      return {
        ok: items.every((item) => item.status === 'completed'),
        errors: items.filter((item) => item.error).map((item) => item.error?.message ?? 'error'),
      }
    }
    await new Promise((resolve) => window.setTimeout(resolve, 200))
  }
  return { ok: false, errors: ['timeout'] }
}

async function runTransferSelftest(sessionId: string): Promise<void> {
  const dev = info.value?.dev
  const localPath = dev?.transferLocalPath
  const remoteDir = dev?.transferRemoteDir
  const downloadDir = dev?.transferDownloadDir
  if (!localPath || !remoteDir || !downloadDir) {
    ElMessage.error('传输自检缺少 NESTTERM_DEV_TRANSFER_* 环境变量')
    return
  }

  const report: Record<string, unknown> = { steps: [] as string[], ok: false }
  const steps = report.steps as string[]
  const remoteName = localPath.split('/').pop() ?? 'transfer-test.bin'
  const remotePath = `${remoteDir}/${remoteName}`

  try {
    steps.push('upload overwrite')
    const first = await sftpUpload({
      sessionId,
      localPaths: [localPath],
      remoteDir,
      conflictPolicy: 'overwrite',
    })
    const firstWait = await waitTransfers(first.transferIds)
    report.uploadOk = firstWait.ok
    if (!firstWait.ok) steps.push(`upload failed: ${firstWait.errors.join('; ')}`)

    steps.push('conflict ask')
    try {
      await sftpUpload({
        sessionId,
        localPaths: [localPath],
        remoteDir,
        conflictPolicy: 'ask',
      })
      report.conflictDetected = false
    } catch (error) {
      report.conflictDetected = devErrorCode(error) === 'SFTP_CONFLICT'
    }

    steps.push('upload overwrite again')
    const second = await sftpUpload({
      sessionId,
      localPaths: [localPath],
      remoteDir,
      conflictPolicy: 'overwrite',
    })
    report.uploadOverwriteOk = (await waitTransfers(second.transferIds)).ok

    steps.push('download')
    const download = await sftpDownload({
      sessionId,
      remotePaths: [remotePath],
      localDir: downloadDir,
      conflictPolicy: 'overwrite',
    })
    report.downloadOk = (await waitTransfers(download.transferIds)).ok
    report.localDir = download.localDir ?? downloadDir

    report.ok =
      report.uploadOk === true &&
      report.conflictDetected === true &&
      report.uploadOverwriteOk === true &&
      report.downloadOk === true
    steps.push(report.ok ? 'ok' : 'FAILED')
  } catch (error) {
    report.error = errorMessage(error)
    report.detail = errorDetail(error)
    steps.push(`FAIL: ${errorMessage(error)}`)
  }

  try {
    await sftpWriteFile({
      sessionId,
      path: '/home/nestterm/nestterm-transfer-selftest.json',
      content: bytesToB64(new TextEncoder().encode(JSON.stringify(report, null, 2))),
    })
  } catch (error) {
    console.error('[nestterm] 写入传输自检报告失败：', error)
  }

  if (report.ok === true) {
    ElMessage.success('传输自检通过：上传/冲突/覆盖/下载')
  } else {
    ElMessage.error('传输自检失败，详见 ~/nestterm-transfer-selftest.json')
  }
}

async function runCancelSelftest(sessionId: string): Promise<void> {
  const dev = info.value?.dev
  const localPath = dev?.transferCancelLocal
  const remoteDir = dev?.transferRemoteDir
  if (!localPath || !remoteDir) {
    ElMessage.error('取消自检缺少 NESTTERM_DEV_TRANSFER_CANCEL_LOCAL / REMOTE_DIR')
    return
  }
  const report: Record<string, unknown> = { ok: false, steps: [] as string[] }
  const steps = report.steps as string[]
  const name = localPath.split('/').pop() ?? 'cancel-test.bin'

  try {
    const started = await sftpUpload({
      sessionId,
      localPaths: [localPath],
      remoteDir,
      conflictPolicy: 'overwrite',
    })
    const id = started.transferIds[0]
    if (!id) throw new Error('没有创建传输任务')
    steps.push('started')
    await new Promise((resolve) => window.setTimeout(resolve, 80))
    await sftpCancelTransfer(id)
    steps.push('cancel requested')

    await waitTransfers([id], 60_000)
    const item = transferItems.value.find((entry) => entry.transferId === id)
    report.status = item?.status ?? 'missing'
    report.cancelled = item?.status === 'cancelled'

    const listing = await sftpList(sessionId, remoteDir)
    report.remoteFileExists = listing.entries.some((entry) => entry.name === name)
    report.partLeftovers = listing.entries
      .filter((entry) => entry.name.includes('.nestterm-') && entry.name.endsWith('.part'))
      .map((entry) => entry.name)

    report.ok =
      report.cancelled === true &&
      report.remoteFileExists === false &&
      (report.partLeftovers as string[]).length === 0
    steps.push(report.ok ? 'ok' : 'FAILED')
  } catch (error) {
    report.error = errorMessage(error)
    steps.push(`FAIL: ${errorMessage(error)}`)
  }

  try {
    await sftpWriteFile({
      sessionId,
      path: '/home/nestterm/nestterm-cancel-selftest.json',
      content: bytesToB64(new TextEncoder().encode(JSON.stringify(report, null, 2))),
    })
  } catch (error) {
    console.error('[nestterm] 写入取消自检报告失败：', error)
  }

  if (report.ok === true) ElMessage.success('取消自检通过：任务已取消且无临时文件残留')
  else ElMessage.error('取消自检失败，详见 ~/nestterm-cancel-selftest.json')
}

async function exportConnections(): Promise<void> {
  try {
    const result = await nesttermExport()
    ElMessage.success(
      `已导出 ${result.profiles} 个连接 / ${result.groups} 个分组（不含密码）\n${result.path}`,
    )
  } catch (error) {
    ElMessage.error(errorMessage(error))
  }
}

async function runExportTest(): Promise<void> {
  try {
    const result = await nesttermExport(info.value?.dev?.exportPath ?? undefined)
    ElMessage.success(`导出自检：${result.profiles} 个连接 / ${result.groups} 个分组 → ${result.path}`)
  } catch (error) {
    ElMessage.error(`导出自检失败：${errorMessage(error)}`)
  }
}

async function importNesttermJson(): Promise<void> {
  try {
    const result = await ElMessageBox.prompt('NestTerm 导出 JSON 的绝对路径', '导入 JSON 备份', {
      confirmButtonText: '导入',
      cancelButtonText: '取消',
      inputPlaceholder: '~/Downloads/NestTerm/nestterm-export-xxxx.json',
    })
    const imported = await nesttermImport(result.value.trim())
    await refreshProfiles()
    ElMessage.success(
      `导入 ${imported.profilesCreated} 个连接 / ${imported.groupsCreated} 个分组，跳过 ${imported.skipped} 个`,
    )
    if (imported.warnings.length > 0) {
      await ElMessageBox.alert(imported.warnings.join('\n'), '导入提示', {
        confirmButtonText: '知道了',
      }).catch(() => undefined)
    }
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function runJsonImportAuto(path: string): Promise<void> {
  try {
    const imported = await nesttermImport(path)
    await refreshProfiles()
    ElMessage.success(
      `JSON 导入自检：${imported.profilesCreated} 连接 / ${imported.groupsCreated} 分组 / 跳过 ${imported.skipped}`,
    )
  } catch (error) {
    ElMessage.error(`JSON 导入自检失败：${errorMessage(error)}`)
  }
}

async function runImportTest(): Promise<void> {
  try {
    const preview = await finalshellPreview()
    const result = await finalshellImport()
    const second = await finalshellImport()
    await refreshProfiles()
    ElMessage.success(
      `导入自检：预览 ${preview.connections}，首次创建 ${result.created}，二次导入跳过 ${second.skipped}`,
    )
  } catch (error) {
    ElMessage.error(`导入自检失败：${errorMessage(error)}`)
  }
}

async function importFinalshell(): Promise<void> {
  try {
    const preview = await finalshellPreview()
    const warningPreview = preview.warnings
      .slice(0, 3)
      .map((line: string) => `· ${line}`)
      .join('\n')
    const message = [
      `目录：${preview.dir}`,
      `发现 ${preview.connections} 个连接（${preview.withPassword} 个含加密密码，不会导入）。`,
      warningPreview ? `\n${warningPreview}\n` : '',
      '导入到「FinalShell 导入」分组？',
    ].join('\n')
    await ElMessageBox.confirm(message, '导入 FinalShell', {
      confirmButtonText: '导入',
      cancelButtonText: '取消',
      type: 'info',
    })
    const result = await finalshellImport()
    await refreshProfiles()
    ElMessage.success(`已导入 ${result.created} 个连接，跳过 ${result.skipped} 个`)
    if (result.warnings.length > 0) {
      await ElMessageBox.alert(result.warnings.join('\n'), '导入提示', {
        confirmButtonText: '知道了',
      }).catch(() => undefined)
    }
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function runForwardTest(profileId: string, sessionId: string): Promise<void> {
  try {
    let rules = await forwardRulesList(profileId)
    const ensure = async (
      name: string,
      kind: 'local' | 'dynamic' | 'remote',
      bindPort: number,
      targetHost?: string,
      targetPort?: number,
    ) => {
      let rule = rules.find((item) => item.name === name)
      if (!rule) {
        rule = await forwardRuleCreate({
          profileId,
          name,
          kind,
          bindHost: '127.0.0.1',
          bindPort,
          targetHost: kind === 'dynamic' ? null : targetHost,
          targetPort: kind === 'dynamic' ? null : targetPort,
        })
        rules.push(rule)
      }
      return rule
    }
    const local = await ensure('Dev local 2222', 'local', 2222, '127.0.0.1', 22)
    const socks = await ensure('Dev SOCKS 1080', 'dynamic', 1080)
    const remote = await ensure('Dev remote 9000', 'remote', 9000, '127.0.0.1', 18080)
    const running = new Set(await forwardRunning())
    for (const rule of [local, socks, remote]) {
      if (!running.has(rule.id)) await forwardStart(sessionId, rule.id)
    }
    ElMessage.success('转发自检：2222→SSH，1080→SOCKS5，远端 9000→本地 18080')
  } catch (error) {
    ElMessage.error(`转发自检失败：${errorMessage(error)}`)
  }
}

async function maybeAutoConnect(): Promise<void> {
  // 仅 Vite dev + debug 后端会带 dev 字段，release 永远不执行。
  if (!import.meta.env.DEV) return
  const auto = info.value?.dev?.autoConnect
  if (!auto) return
  const profile = profileState.profiles.find(
    (item) => item.id === auto || item.name === auto,
  )
  if (!profile) {
    ElMessage.error(`联调自动化：找不到连接「${auto}」`)
    return
  }
  pendingDevCommand = info.value?.dev?.initialCommand ?? null
  const inlinePassword = info.value?.dev?.autoConnectPassword ?? undefined
  await connectProfile(profile, inlinePassword ? { inlinePassword } : undefined)
  if (info.value?.dev?.forwardTest) {
    const tab = activeTab.value
    if (tab?.profileId && tab.status === 'connected') {
      await runForwardTest(tab.profileId, tab.sessionId)
    }
  }
}

watch(tabs, saveOpenProfiles, { deep: true })

onBeforeUnmount(() => {
  unlisteners.splice(0).forEach((off) => off())
  window.removeEventListener('keydown', onKeydown)
})

function onKeydown(event: KeyboardEvent): void {
  const meta = event.metaKey || event.ctrlKey
  if (!meta) return
  const key = event.key.toLowerCase()
  if (key === 'k') {
    event.preventDefault()
    treeRef.value?.focusSearch()
  } else if (key === 't') {
    event.preventDefault()
    openCreate(null)
  }
}

function openCreate(groupId: string | null): void {
  dialogProfile.value = null
  presetGroupId.value = groupId
  dialogMode.value = 'create'
  dialogVisible.value = true
}

function openEdit(profile: ConnProfile): void {
  dialogProfile.value = profile
  presetGroupId.value = null
  dialogMode.value = 'edit'
  dialogVisible.value = true
}

function openConnectDialog(profile: ConnProfile): void {
  dialogProfile.value = profile
  presetGroupId.value = null
  dialogMode.value = 'connect'
  dialogVisible.value = true
}

async function connectProfile(
  profile: ConnProfile,
  inline?: { inlinePassword?: string; inlinePassphrase?: string },
): Promise<void> {
  const existing = findTabByProfile(profile.id)
  if (existing) {
    activateTab(existing.sessionId)
    return
  }

  const sessionId = crypto.randomUUID()
  addTab({
    sessionId,
    profileId: profile.id,
    title: profile.name,
    status: 'connecting',
    message: '正在建立连接…',
    fingerprint: '',
    cwd: null,
    cwdSource: null,
    home: null,
  })

  // 等终端挂载后再取真实行列，避免首屏 PTY 尺寸错误。
  await nextTick()
  const { cols, rows } = getTerminalSize(sessionId)

  let auth: AuthInput | undefined
  if (inline?.inlinePassword) {
    auth = { method: 'password', source: 'inline', password: inline.inlinePassword }
  } else if (inline?.inlinePassphrase && profile.keyPath) {
    auth = {
      method: 'key',
      source: 'inline',
      keyPath: profile.keyPath,
      passphrase: inline.inlinePassphrase,
    }
  }

  try {
    await sshConnect({ sessionId, profileId: profile.id, auth, cols, rows })
    // 用户可能在握手期间就关掉了标签：此时后端已建连，必须立刻回收，避免孤儿会话。
    if (!tabs.value.some((item) => item.sessionId === sessionId)) {
      await sshDisconnect(sessionId).catch(() => undefined)
      return
    }

    if (pendingDevCommand) {
      const command = pendingDevCommand
      pendingDevCommand = null
      // 给远端 shell 一点时间进入交互态，再发送测试命令。
      await new Promise((resolve) => window.setTimeout(resolve, 400))
      await sshWrite(sessionId, bytesToB64(new TextEncoder().encode(`${command}\n`)))
    }

    if (info.value?.dev?.sftpSelftest) {
      window.setTimeout(() => {
        void runSftpSelftest(sessionId)
      }, 1200)
    }

    if (info.value?.dev?.transferSelftest) {
      window.setTimeout(() => {
        void runTransferSelftest(sessionId)
      }, 1200)
    }

    if (info.value?.dev?.transferCancelLocal) {
      window.setTimeout(() => {
        void runCancelSelftest(sessionId)
      }, 1200)
    }
  } catch (error) {
    const tab = tabs.value.find((item) => item.sessionId === sessionId)
    if (!tab) return
    const message = errorMessage(error)
    tab.status = 'error'
    tab.message = message
    reportError(sessionId, message)
  }
}

function openProfile(profile: ConnProfile): void {
  const existing = findTabByProfile(profile.id)
  if (existing) {
    activateTab(existing.sessionId)
    return
  }
  if (profile.authType === 'password' && !profile.hasPassword) {
    openConnectDialog(profile)
    return
  }
  void connectProfile(profile)
}

function onDialogConnect(payload: {
  profile: ConnProfile
  inlinePassword?: string
  inlinePassphrase?: string
}): void {
  void connectProfile(payload.profile, payload)
}

function onProfileSaved(profile: ConnProfile): void {
  updateTabTitle(profile.id, profile.name)
  void refreshProfiles()
}

async function createGroup(parentId: string | null): Promise<void> {
  try {
    const result = await ElMessageBox.prompt('分组名称', '新建分组', {
      confirmButtonText: '创建',
      cancelButtonText: '取消',
      inputValidator: (value) => (value && value.trim() ? true : '请输入分组名称'),
    })
    await groupCreate(parentId, result.value.trim())
    await refreshProfiles()
    ElMessage.success('分组已创建')
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function renameGroup(group: Group): Promise<void> {
  try {
    const result = await ElMessageBox.prompt('分组名称', '重命名分组', {
      confirmButtonText: '保存',
      cancelButtonText: '取消',
      inputValue: group.name,
      inputValidator: (value) => (value && value.trim() ? true : '请输入分组名称'),
    })
    await groupUpdate(group.id, result.value.trim())
    await refreshProfiles()
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function removeGroup(group: Group): Promise<void> {
  const count = profileState.profiles.filter((profile) => profile.groupId === group.id).length
  try {
    await ElMessageBox.confirm(
      `删除分组「${group.name}」？其中 ${count} 个连接会移到未分组，子分组会移到上一级。`,
      '删除分组',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
    await groupDelete(group.id, 'move-to-parent')
    await refreshProfiles()
    ElMessage.success('分组已删除')
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function removeProfile(profile: ConnProfile): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `删除连接「${profile.name}」？已保存的密码/口令会同时从系统钥匙串删除。`,
      '删除连接',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
    await profileDelete([profile.id])
    const tab = findTabByProfile(profile.id)
    if (tab) {
      try {
        await sshDisconnect(tab.sessionId)
      } catch {
        /* 已断开时忽略 */
      }
      removeTab(tab.sessionId)
    }
    await refreshProfiles()
    ElMessage.success('连接已删除')
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(errorMessage(error))
  }
}

async function duplicateProfile(profile: ConnProfile): Promise<void> {
  try {
    const copy = await profileDuplicate(profile.id)
    await refreshProfiles()
    ElMessage.success(`已创建「${copy.name}」，请重新输入凭据`)
  } catch (error) {
    ElMessage.error(errorMessage(error))
  }
}
</script>

<template>
  <el-config-provider :locale="zhCn">
  <div class="app-shell">
    <header class="app-header">
      <div class="brand">
        <span class="brand-mark">N</span>
        <div class="brand-copy">
          <h1>NestTerm</h1>
          <p>SSH / SFTP 运维客户端 · Tauri 2 + Rust</p>
        </div>
      </div>

      <div class="header-actions">
        <el-button size="small" type="primary" @click="openCreate(null)">＋ 连接</el-button>
        <el-button size="small" @click="createGroup(null)">＋ 分组</el-button>
        <el-button size="small" title="扫描 FinalShell 本地连接并导入" @click="importFinalshell">
          导入
        </el-button>
        <el-button size="small" title="导出 NestTerm JSON（不含密码）" @click="exportConnections">
          导出
        </el-button>
        <el-button size="small" title="导入 NestTerm JSON 备份" @click="importNesttermJson">
          导入 JSON
        </el-button>
        <el-button size="small" title="管理主机指纹" @click="knownHostsVisible = true">指纹</el-button>
        <el-button size="small" title="设置" @click="settingsVisible = true">设置</el-button>
      </div>

      <div class="header-meta">
        <el-tag v-if="info" size="small" effect="plain" type="info">
          {{ info.arch }} · {{ info.os }}
        </el-tag>
        <el-tag size="small" effect="dark" :type="statusTagType">{{ statusLabel }}</el-tag>
      </div>
    </header>

    <el-alert
      v-if="startupError"
      class="startup-alert"
      type="error"
      :closable="false"
      show-icon
      :title="`存储初始化失败：${startupError.message}`"
    >
      <template #default>
        <p class="startup-hint">
          连接配置与系统钥匙串当前不可用。请检查 macOS 钥匙串访问权限后重启应用。
        </p>
        <pre v-if="startupDetail" class="startup-detail">{{ startupDetail }}</pre>
      </template>
    </el-alert>

    <main class="app-body">
      <aside class="sidebar">
        <SessionTree
          ref="treeRef"
          @open="openProfile"
          @edit="openEdit"
          @duplicate="duplicateProfile"
          @remove="removeProfile"
          @new-profile="openCreate"
          @new-group="createGroup"
          @rename-group="renameGroup"
          @remove-group="removeGroup"
        />
      </aside>

      <section class="workspace">
        <WorkspaceTabs
          v-if="tabs.length > 0"
          :initial-monitor="info?.dev?.monitorPanel ?? false"
          @new="openCreate(null)"
        />
        <div v-else class="welcome">
          <div class="welcome-mark">N</div>
          <h2>NestTerm</h2>
          <p class="welcome-sub">原生 arm64 · SSH / SFTP 运维客户端</p>
          <div class="welcome-actions">
            <el-button type="primary" @click="openCreate(null)">新建连接</el-button>
            <el-button @click="createGroup(null)">新建分组</el-button>
            <el-button @click="importFinalshell">导入 FinalShell</el-button>
          </div>
          <ul class="welcome-hints">
            <li><kbd>⌘K</kbd> 搜索连接</li>
            <li><kbd>⌘T</kbd> 新建连接</li>
            <li>双击左侧连接开始会话</li>
          </ul>
          <p class="welcome-note">
            B1–B3 已实现：SQLCipher 加密存储、多标签终端、SFTP 文件/目录传输、FinalShell 导入、
            本地/远端转发、SOCKS5、系统监控与自动重连。
          </p>
        </div>
      </section>
    </main>

    <SettingsDialog v-model="settingsVisible" />
    <KnownHostsDialog v-model="knownHostsVisible" />

    <footer class="statusbar">
      <span class="status-dot" :class="`dot-${activeTab?.status ?? 'idle'}`" />
      <span>{{ statusLabel }}</span>
      <span v-if="activeTab?.message" class="status-message">{{ activeTab.message }}</span>
      <span class="spacer" />
      <span>{{ profileState.profiles.length }} 个连接</span>
      <span v-if="info">{{ info.arch }} · schema v{{ info.dbVersion }}</span>
    </footer>

    <ProfileDialog
      v-model:visible="dialogVisible"
      :groups="profileState.groups"
      :profile="dialogProfile"
      :preset-group-id="presetGroupId"
      :mode="dialogMode"
      @saved="onProfileSaved"
      @connect="onDialogConnect"
    />
  </div>
  </el-config-provider>
</template>
