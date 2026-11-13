import { computed, ref } from 'vue'
import type { SshStatusPayload, SshStatusState } from '../lib/types'

/** 会话标签与终端写入通道（轻量单例，不引入 Pinia）。 */

export interface SessionTab {
  sessionId: string
  profileId: string | null
  title: string
  status: SshStatusState
  message: string
  fingerprint: string
  /** 终端 OSC 7 / cd 启发式上报的 cwd；SFTP 跟随用。 */
  cwd: string | null
  cwdSource: 'prompt' | 'manual' | null
  home: string | null
}

export const tabs = ref<SessionTab[]>([])
export const activeId = ref<string | null>(null)

/** xterm 写入器：TerminalPane 挂载后注册；挂载前的输出先入缓冲，避免丢首屏。 */
const writers = new Map<string, (b64: string) => void>()
const buffers = new Map<string, string[]>()
const sizeGetters = new Map<string, () => { cols: number; rows: number }>()

export const activeTab = computed(
  () => tabs.value.find((tab) => tab.sessionId === activeId.value) ?? null,
)

export function addTab(tab: SessionTab): void {
  tabs.value = [...tabs.value, tab]
  activeId.value = tab.sessionId
}

export function activateTab(sessionId: string): void {
  activeId.value = sessionId
}

export function removeTab(sessionId: string): void {
  tabs.value = tabs.value.filter((tab) => tab.sessionId !== sessionId)
  writers.delete(sessionId)
  buffers.delete(sessionId)
  sizeGetters.delete(sessionId)
  if (activeId.value === sessionId) {
    activeId.value = tabs.value[tabs.value.length - 1]?.sessionId ?? null
  }
}

export function findTabByProfile(profileId: string | null): SessionTab | undefined {
  if (!profileId) return undefined
  return tabs.value.find((tab) => tab.profileId === profileId)
}

export function updateStatus(payload: SshStatusPayload): void {
  const tab = tabs.value.find((item) => item.sessionId === payload.sessionId)
  if (!tab) return
  tab.status = payload.state
  if (payload.message) tab.message = payload.message
}

export function setFingerprint(sessionId: string, fingerprint: string): void {
  const tab = tabs.value.find((item) => item.sessionId === sessionId)
  if (tab) tab.fingerprint = fingerprint
}

export function setCwd(
  sessionId: string,
  cwd: string,
  source: 'prompt' | 'manual',
): void {
  const tab = tabs.value.find((item) => item.sessionId === sessionId)
  if (tab) {
    tab.cwd = cwd
    tab.cwdSource = source
  }
}

export function setHome(sessionId: string, home: string): void {
  const tab = tabs.value.find((item) => item.sessionId === sessionId)
  if (tab) tab.home = home
}

export function updateTabTitle(profileId: string, title: string): void {
  const tab = tabs.value.find((item) => item.profileId === profileId)
  if (tab) tab.title = title
}

export function registerWriter(sessionId: string, writer: (b64: string) => void): () => void {
  writers.set(sessionId, writer)
  const pending = buffers.get(sessionId)
  if (pending?.length) {
    buffers.delete(sessionId)
    for (const chunk of pending) writer(chunk)
  }
  return () => {
    if (writers.get(sessionId) === writer) writers.delete(sessionId)
  }
}

export function writeTerminal(sessionId: string, b64: string): void {
  const writer = writers.get(sessionId)
  if (writer) {
    writer(b64)
    return
  }
  // 终端尚未挂载：缓冲少量数据（首屏提示符最多几 KB）。
  const pending = buffers.get(sessionId) ?? []
  pending.push(b64)
  if (pending.length > 256) pending.splice(0, pending.length - 256)
  buffers.set(sessionId, pending)
}

export function registerSize(
  sessionId: string,
  getter: () => { cols: number; rows: number },
): () => void {
  sizeGetters.set(sessionId, getter)
  return () => {
    if (sizeGetters.get(sessionId) === getter) sizeGetters.delete(sessionId)
  }
}

export function getTerminalSize(sessionId: string): { cols: number; rows: number } {
  return sizeGetters.get(sessionId)?.() ?? { cols: 120, rows: 32 }
}
