import { ref } from 'vue'
import type { ApiError, TransferDonePayload, TransferProgressPayload } from '../lib/types'

/** 传输任务列表：进度事件驱动，完成/失败保留供查看。 */

export interface TransferItem extends TransferProgressPayload {
  status: 'running' | 'completed' | 'cancelled' | 'failed'
  error?: ApiError
}

export const transferItems = ref<TransferItem[]>([])

export function upsertTransfer(payload: TransferProgressPayload): void {
  const existing = transferItems.value.find((item) => item.transferId === payload.transferId)
  if (existing) {
    Object.assign(existing, payload)
    return
  }
  const item: TransferItem = { ...payload, status: 'running' }
  transferItems.value = [...transferItems.value, item].slice(-60)
}

export function finishTransfer(payload: TransferDonePayload): void {
  const item = transferItems.value.find((entry) => entry.transferId === payload.transferId)
  if (!item) return
  item.status = payload.status
  if (payload.error) item.error = payload.error
  if (payload.status === 'completed') {
    item.bytes = item.totalBytes
  }
}

export function transfersForSession(sessionId: string): TransferItem[] {
  return transferItems.value
    .filter((item) => item.sessionId === sessionId)
    .slice(-10)
    .reverse()
}

export function activeTransferCount(sessionId: string): number {
  return transferItems.value.filter(
    (item) => item.sessionId === sessionId && item.status === 'running',
  ).length
}

export function pruneFinishedTransfers(sessionId: string): void {
  transferItems.value = transferItems.value.filter(
    (item) => item.sessionId !== sessionId || item.status === 'running',
  )
}
