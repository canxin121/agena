import { isValidPromptFragmentId, normalizePromptFragment, normalizePromptLayer } from './promptLayers'
import type { PromptLayer } from './promptLayers'
import type { JsonObject } from '../types/json'

export type AgentProfileConfig = {
  title?: string
  description?: string
  extends?: string | null
  model?: JsonObject | null
  prompts?: Partial<PromptLayer>
  permission?: JsonObject
  max_turns?: number | null
}

export type AgentProfileCatalog = Record<string, AgentProfileConfig>

export function agentProfileCatalog(value: unknown): AgentProfileCatalog {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  // Settings values can be Vue proxies; JSON data must be copied without
  // structuredClone's rejection of reactive objects.
  return JSON.parse(JSON.stringify(value)) as AgentProfileCatalog
}

/** A workspace entry replaces the entire global entry with the same name. */
export function mergedAgentProfiles(global: AgentProfileCatalog, workspace: AgentProfileCatalog): AgentProfileCatalog {
  return { ...global, ...workspace }
}

export function isValidAgentProfileName(name: string): boolean {
  return isValidPromptFragmentId(name)
}

export function agentProfileDisplayName(name: string, profile: AgentProfileConfig): string {
  return profile.title?.trim() || name
}

export function agentProfileLineage(catalog: AgentProfileCatalog, name: string): string[] {
  const chain: string[] = []
  let current: string | null | undefined = name
  while (current) {
    if (chain.includes(current)) throw new Error(`Profile inheritance cycle: ${[...chain, current].join(' → ')}`)
    const profile: AgentProfileConfig | undefined = Object.hasOwn(catalog, current) ? catalog[current] : undefined
    if (!profile) throw new Error(`Unknown profile: ${current}`)
    chain.push(current)
    current = profile.extends
  }
  return chain.reverse()
}

/** Reuse an existing single inline instruction, preserving its id and conditions. */
function instructionId(name: string, layer: PromptLayer): string {
  if (layer.inline.length === 1) return layer.inline[0]!.id
  let hash = 2166136261
  for (const character of name) hash = Math.imul(hash ^ character.charCodeAt(0), 16777619)
  const preferred =
    name.length <= 56 ? `profile-${name}` : `profile-${name.slice(0, 47)}-${(hash >>> 0).toString(16).padStart(8, '0')}`
  return preferred
}

export function agentProfileInstructions(name: string, profile: AgentProfileConfig): string {
  const layer = normalizePromptLayer(profile.prompts)
  return layer.inline.find((fragment) => fragment.id === instructionId(name, layer))?.body || ''
}

/** Editing the simple body must not discard saved references or advanced layers. */
export function withAgentProfileInstructions(
  name: string,
  profile: AgentProfileConfig,
  body: string,
): AgentProfileConfig {
  const layer = normalizePromptLayer(profile.prompts)
  const id = instructionId(name, layer)
  const previous = layer.inline.find((fragment) => fragment.id === id)
  if (!body.trim()) {
    layer.inline = layer.inline.filter((fragment) => fragment.id !== id)
    layer.order = layer.order.filter((entry) => entry !== id)
  } else {
    const fragment = normalizePromptFragment({ ...previous, id, body })
    layer.inline = [...layer.inline.filter((entry) => entry.id !== id), fragment]
    if (!layer.order.includes(id)) layer.order.push(id)
    layer.disabled = layer.disabled.filter((entry) => entry !== id)
  }
  return { ...profile, prompts: layer }
}

export function parseAgentProfileObject(text: string): JsonObject {
  const value: unknown = JSON.parse(text.trim() || '{}')
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Expected a JSON object')
  return value as JsonObject
}

/** Removing an entry is safe only once its direct configuration references are moved. */
export function agentProfileReferences(catalog: AgentProfileCatalog, name: string): string[] {
  return Object.entries(catalog)
    .filter(([, profile]) => profile.extends === name)
    .map(([id]) => id)
}
