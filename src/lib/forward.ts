import { invoke } from '@tauri-apps/api/core'

export interface ForwardRule {
  id: string
  profileId: string
  name: string
  kind: 'local' | 'dynamic' | 'remote'
  bindHost: string
  bindPort: number
  targetHost: string | null
  targetPort: number | null
  autoStart: boolean
  createdAt: number
  updatedAt: number
}

export interface ForwardRuleInput {
  profileId: string
  name: string
  kind: 'local' | 'dynamic' | 'remote'
  bindHost: string
  bindPort: number
  targetHost?: string | null
  targetPort?: number | null
  autoStart?: boolean
}

export const forwardRulesList = (profileId: string) =>
  invoke<ForwardRule[]>('forward_rules_list', { profileId })

export const forwardRuleCreate = (input: ForwardRuleInput) =>
  invoke<ForwardRule>('forward_rule_create', { input })

export const forwardRuleDelete = (id: string) => invoke<void>('forward_rule_delete', { id })

export const forwardStart = (sessionId: string, ruleId: string) =>
  invoke<void>('forward_start', { sessionId, ruleId })

export const forwardStop = (ruleId: string) => invoke<void>('forward_stop', { ruleId })

export const forwardRunning = () => invoke<string[]>('forward_running')
