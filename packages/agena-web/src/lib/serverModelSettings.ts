import type { JsonObject, JsonValue } from '@/types/json'

export type ServerModelIdentity = {
  provider: string
  adapter: string
  model: string
}

export type ServerModelModes = {
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function asRecord(value: JsonValue): JsonObject | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as JsonObject
}

export function normalizeServerModelIdentity(
  value:
    | {
        provider?: unknown
        provider_id?: unknown
        adapter?: unknown
        adapter_id?: unknown
        model?: unknown
        model_id?: unknown
      }
    | null
    | undefined,
): ServerModelIdentity {
  return {
    provider: text(value?.provider) || text(value?.provider_id),
    adapter: text(value?.adapter) || text(value?.adapter_id),
    model: text(value?.model) || text(value?.model_id),
  }
}

export function sameServerModelIdentity(
  left: Partial<ServerModelIdentity> | null | undefined,
  right: Partial<ServerModelIdentity> | null | undefined,
): boolean {
  const normalizedLeft = normalizeServerModelIdentity(left)
  const normalizedRight = normalizeServerModelIdentity(right)
  return (
    normalizedLeft.provider === normalizedRight.provider &&
    normalizedLeft.adapter === normalizedRight.adapter &&
    normalizedLeft.model === normalizedRight.model
  )
}

function modelSelectionValue(identity: ServerModelIdentity, modes?: ServerModelModes): JsonObject {
  const normalized = normalizeServerModelIdentity(identity)
  if (!normalized.provider || !normalized.model) {
    throw new Error('A provider and model are required.')
  }
  const thinkingMode = text(modes?.thinkingMode)
  const speedMode = text(modes?.speedMode)
  const verbosity = text(modes?.verbosity)
  return {
    provider: normalized.provider,
    ...(normalized.adapter ? { adapter: normalized.adapter } : {}),
    model: normalized.model,
    ...(thinkingMode ? { thinking_mode: thinkingMode } : {}),
    ...(speedMode ? { speed_mode: speedMode } : {}),
    ...(verbosity ? { verbosity } : {}),
    ...(typeof modes?.parallelToolCalls === 'boolean' ? { parallel_tool_calls: modes.parallelToolCalls } : {}),
  }
}

export function buildProviderDefaultSettingsPatch(identity: ServerModelIdentity, modes?: ServerModelModes): JsonObject {
  const selection = modelSelectionValue(identity, modes)
  return {
    path: 'providers',
    changes: {
      default: selection.provider,
      default_selection: selection,
    },
    dry_run: false,
    validate: true,
    reload: true,
  }
}

export function buildApprovalModelSettingsPatch(
  identity: ServerModelIdentity | null,
  modes?: ServerModelModes,
): JsonObject {
  let approvalModel: JsonValue = null
  if (identity) {
    const selection = modelSelectionValue(identity, modes)
    approvalModel = {
      provider_id: selection.provider,
      ...(selection.adapter ? { adapter_id: selection.adapter } : {}),
      model_id: selection.model,
      ...(selection.thinking_mode ? { thinking_mode: selection.thinking_mode } : {}),
      ...(selection.speed_mode ? { speed_mode: selection.speed_mode } : {}),
      ...(selection.verbosity ? { verbosity: selection.verbosity } : {}),
      ...(typeof selection.parallel_tool_calls === 'boolean'
        ? { parallel_tool_calls: selection.parallel_tool_calls }
        : {}),
    }
  }
  return {
    path: 'permission',
    changes: { approval_model: approvalModel },
    dry_run: false,
    validate: true,
    reload: true,
  }
}

export function approvalModelFromSettingsResponse(payload: JsonValue): {
  identity: ServerModelIdentity
  modes: ServerModelModes
} | null {
  const value = asRecord(asRecord(payload)?.value)
  const identity = normalizeServerModelIdentity(value)
  if (!identity.provider || !identity.model) return null
  const thinkingMode = text(value?.thinking_mode)
  const speedMode = text(value?.speed_mode)
  const verbosity = text(value?.verbosity)
  return {
    identity,
    modes: {
      ...(thinkingMode ? { thinkingMode } : {}),
      ...(speedMode ? { speedMode } : {}),
      ...(verbosity ? { verbosity } : {}),
      ...(typeof value?.parallel_tool_calls === 'boolean' ? { parallelToolCalls: value.parallel_tool_calls } : {}),
    },
  }
}
