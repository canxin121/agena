import { encodeModelSelectionKey, parseModelSlug } from './modelSelectionDefaults'
import type { SessionRunConfig } from '@/types/chat'

type ChatMessageLike = {
  info?: {
    providerID?: unknown
    adapterID?: unknown
    modelID?: unknown
  }
}

export type SessionSelection = {
  provider: string
  adapter: string
  model: string
  thinkingMode: string
  speedMode: string
  verbosity: string
  parallelToolCalls?: boolean
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function readSessionRunConfigSelection(runConfig: SessionRunConfig | null | undefined): SessionSelection {
  return {
    provider: text(runConfig?.providerID),
    adapter: text(runConfig?.adapterID),
    model: text(runConfig?.modelID),
    thinkingMode: text(runConfig?.thinkingMode),
    speedMode: text(runConfig?.speedMode),
    verbosity: text(runConfig?.verbosity),
    ...(typeof runConfig?.parallelToolCalls === 'boolean' ? { parallelToolCalls: runConfig.parallelToolCalls } : {}),
  }
}

export function deriveSessionSelectionFromMessages(messages: ChatMessageLike[]): SessionSelection {
  const list = Array.isArray(messages) ? messages : []
  for (let index = list.length - 1; index >= 0; index -= 1) {
    const info = list[index]?.info
    const provider = text(info?.providerID)
    const model = text(info?.modelID)
    if (!provider || !model) continue
    return {
      provider,
      adapter: text(info?.adapterID),
      model,
      thinkingMode: '',
      speedMode: '',
      verbosity: '',
    }
  }
  return {
    provider: '',
    adapter: '',
    model: '',
    thinkingMode: '',
    speedMode: '',
    verbosity: '',
  }
}

export function normalizeSessionManualModelStorageEntry(
  sessionId: string,
  value: unknown,
): { key: string; value: string } | null {
  const key = text(sessionId)
  if (!key || typeof value !== 'string') return null
  const selection = parseModelSlug(value)
  const normalized = encodeModelSelectionKey(selection)
  return normalized ? { key, value: normalized } : null
}

export function readSessionManualModelPair(
  map: Record<string, string>,
  sessionId: string,
): { provider: string; adapter: string; model: string } {
  const key = text(sessionId)
  return key ? parseModelSlug(map[key] || '') : { provider: '', adapter: '', model: '' }
}

export function writeSessionManualModelPair(
  map: Record<string, string>,
  sessionId: string,
  provider: string,
  adapter: string,
  model: string,
): Record<string, string> {
  const key = text(sessionId)
  const value = encodeModelSelectionKey({ provider, adapter, model })
  if (!key || !value || map[key] === value) return map
  return { ...map, [key]: value }
}

export function removeSessionManualModelPair(map: Record<string, string>, sessionId: string): Record<string, string> {
  const key = text(sessionId)
  if (!key || !Object.prototype.hasOwnProperty.call(map, key)) return map
  const next = { ...map }
  delete next[key]
  return next
}
