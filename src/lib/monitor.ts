import { invoke } from '@tauri-apps/api/core'

export interface MonitorStats {
  host: string
  os: string
  uptimeSecs: number
  load1: number
  load5: number
  load15: number
  cpuPercent: number
  memTotal: number
  memUsed: number
  memPercent: number
  swapTotal: number
  swapUsed: number
  diskTotal: number
  diskUsed: number
  diskPercent: number
  diskMount: string
  netRxBytes: number
  netTxBytes: number
  sampledAt: number
}

export const monitorSample = (sessionId: string) =>
  invoke<MonitorStats>('monitor_sample', { sessionId })
