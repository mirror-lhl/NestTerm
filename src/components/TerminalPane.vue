<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { ElMessage } from 'element-plus'
import { b64ToBytes, bytesToB64 } from '../lib/base64'
import { sshResize, sshWrite } from '../lib/ssh'
import type { SshStatusState } from '../lib/types'
import { registerSize, registerWriter, setCwd, tabs } from '../stores/sessions'
import { appSettings } from '../stores/settings'

const props = defineProps<{
  sessionId: string
  status: SshStatusState
  active: boolean
}>()

const host = ref<HTMLDivElement | null>(null)
const term = shallowRef<Terminal | null>(null)
const fitAddon = new FitAddon()

let observer: ResizeObserver | null = null
let rafId = 0
let unregisterWriter: (() => void) | null = null
let unregisterSize: (() => void) | null = null
let promptHintShown = false
let promptAutoFixed = false

function fitNow(): void {
  if (!term.value) return
  try {
    fitAddon.fit()
  } catch {
    /* 容器尚未布局完成，等待下一次 ResizeObserver */
  }
}

function sendResize(): void {
  if (props.status !== 'connected') return
  const size = term.value
  if (!size) return
  void sshResize(props.sessionId, size.cols, size.rows).catch(() => {
    /* 断线瞬间的 resize 失败无需打扰用户 */
  })
}

function focus(): void {
  if (props.active) term.value?.focus()
}

function detectPrompt(instance: Terminal): void {
  const buffer = instance.buffer.active
  // 不只看光标行：shell 重绘/换行时光标行可能不是提示符。
  let trimmed = ''
  const end = Math.max(0, buffer.baseY + buffer.cursorY)
  for (let index = end; index >= Math.max(0, end - 4); index -= 1) {
    const text =
      buffer
        .getLine(index)
        ?.translateToString(true)
        .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
        .trim() ?? ''
    if (/^\[[^\]]+\][#$]$/.test(text) || /^[^@\s]+@[^:\s]+:.+[$#]$/.test(text)) {
      trimmed = text
      break
    }
  }
  if (!trimmed) return
  let raw = ''
  if (trimmed.startsWith('[')) {
    // CentOS 默认：[user@host dir]#；若 PS1 用 \w，dir 就是完整路径。
    const close = trimmed.lastIndexOf(']')
    const inner = trimmed.slice(1, close)
    const space = inner.lastIndexOf(' ')
    raw = inner.slice(space + 1).trim()
  } else {
    const at = trimmed.lastIndexOf('@')
    const colon = trimmed.indexOf(':', at)
    if (at <= 0 || colon <= at) return
    raw = trimmed.slice(colon + 1, -1).trim()
  }
  if (!raw) return
  const tab = tabs.value.find((item) => item.sessionId === props.sessionId)
  let path = raw
  if (raw === '~') {
    if (!tab?.home) return
    path = tab.home
  } else if (raw.startsWith('~/')) {
    if (!tab?.home) return
    path = tab.home + raw.slice(1)
  }
  if (!path.startsWith('/')) {
    if (!promptAutoFixed && props.status === 'connected') {
      // CentOS 默认 \W 只显示目录名。自动把当前会话 PS1 升级为 \w，
      // 只影响本次 shell，不修改远端 .bashrc。
      promptAutoFixed = true
      const command = "PS1='[\\u@\\h \\w]\\$ '\n"
      void sshWrite(props.sessionId, bytesToB64(new TextEncoder().encode(command))).catch(() => undefined)
      return
    }
    if (!promptHintShown) {
      promptHintShown = true
      ElMessage.warning(
        '当前提示符只显示目录名，无法确定完整路径；请在远端 ~/.bashrc 末尾设置 PS1=\'[\\u@\\h \\w]\\$ \'',
      )
    }
    return
  }
  if (tab?.cwd === path && tab.cwdSource === 'prompt') return
  setCwd(props.sessionId, path, 'prompt')
}

onMounted(() => {
  const terminalSettings = appSettings.value?.terminal
  const instance = new Terminal({
    fontFamily:
      terminalSettings?.fontFamily ??
      '"JetBrains Mono", "SF Mono", Menlo, Consolas, "PingFang SC", monospace',
    fontSize: terminalSettings?.fontSize ?? 13,
    lineHeight: terminalSettings?.lineHeight ?? 1.25,
    cursorBlink: terminalSettings?.cursorBlink ?? true,
    cursorStyle: (terminalSettings?.cursorStyle as 'bar' | 'block' | 'underline') ?? 'bar',
    scrollback: terminalSettings?.scrollback ?? 10_000,
    macOptionIsMeta: true,
    theme: {
      background: '#0b0f14',
      foreground: '#d7dee8',
      cursor: '#3ddc97',
      cursorAccent: '#0b0f14',
      selectionBackground: '#2b4358',
      black: '#10161d',
      red: '#f26d78',
      green: '#3ddc97',
      yellow: '#e6c07b',
      blue: '#6ea8fe',
      magenta: '#c792ea',
      cyan: '#67d8ef',
      white: '#d7dee8',
      brightBlack: '#5c6b7a',
      brightRed: '#ff8791',
      brightGreen: '#6ff0b4',
      brightYellow: '#f2d49b',
      brightBlue: '#91bfff',
      brightMagenta: '#dbb2f5',
      brightCyan: '#8ee8f8',
      brightWhite: '#ffffff',
    },
  })

  instance.loadAddon(fitAddon)
  if (host.value) instance.open(host.value)

  instance.onData((chunk) => {
    if (props.status !== 'connected') return
    // 按键按 UTF-8 编码后 base64 传输，中文/粘贴内容不会在 IPC 中被截断。
    const bytes = new TextEncoder().encode(chunk)
    void sshWrite(props.sessionId, bytesToB64(bytes)).catch((error) => {
      console.error('[nestterm] 写入 SSH 失败：', error)
    })
  })
  instance.onResize(() => sendResize())

  term.value = instance
  unregisterWriter = registerWriter(props.sessionId, (b64) => {
    instance.write(b64ToBytes(b64), () => detectPrompt(instance))
  })
  unregisterSize = registerSize(props.sessionId, () => ({
    cols: instance.cols,
    rows: instance.rows,
  }))

  fitNow()
  focus()

  observer = new ResizeObserver(() => {
    cancelAnimationFrame(rafId)
    rafId = requestAnimationFrame(() => {
      fitNow()
    })
  })
  if (host.value) observer.observe(host.value)
})

watch(
  () => appSettings.value?.terminal,
  (settings) => {
    if (!settings || !term.value) return
    term.value.options.fontSize = settings.fontSize
    term.value.options.lineHeight = settings.lineHeight
    term.value.options.scrollback = settings.scrollback
    term.value.options.cursorBlink = settings.cursorBlink
    term.value.options.fontFamily = settings.fontFamily
    fitNow()
    sendResize()
  },
)

onBeforeUnmount(() => {
  observer?.disconnect()
  observer = null
  cancelAnimationFrame(rafId)
  unregisterWriter?.()
  unregisterSize?.()
  term.value?.dispose()
  term.value = null
})

watch(
  () => props.active,
  (active) => {
    if (active) {
      void nextTick(() => {
        fitNow()
        focus()
      })
    }
  },
)

watch(
  () => props.status,
  (status) => {
    if (status === 'connected') {
      void nextTick(() => {
        fitNow()
        sendResize()
        focus()
      })
    }
  },
)
</script>

<template>
  <div ref="host" class="terminal-host" />
</template>

<style scoped>
.terminal-host {
  /* 标签页里所有终端叠放在同一网格位置，由 v-show 控制可见性 */
  position: absolute;
  inset: 0;
  overflow: hidden;
  background: #0b0f14;
}

.terminal-host :deep(.xterm) {
  height: 100%;
  padding: 10px 12px;
  box-sizing: border-box;
}

.terminal-host :deep(.xterm-viewport) {
  background-color: transparent !important;
  scrollbar-width: thin;
}
</style>
