import { isValidPromptFragmentId, normalizePromptFragment, normalizePromptLayer } from './promptLayers'
import type { PromptLayer } from './promptLayers'
import { canonicalPermissionToolEntries, mergeNetworkPermissionConfig, parsePermissionConfig } from './permissionConfig'
import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from './runtimeSettings'
import type { JsonObject } from '../types/json'

export const DEFAULT_PRIMARY_AGENT_PROFILE = 'main'
export const DEFAULT_SUBAGENT_PROFILE = 'worker'

export function resolveAgentProfileDefault(
  bundle: RuntimeSettingsReadBundle | null | undefined,
  layer: RuntimeSettingsLayer,
  fallback: string,
): { name: string; source: 'global' | 'workspace' | 'builtin' } {
  const workspace = bundle?.workspace?.value
  const global = bundle?.global?.value
  if (layer === 'workspace' && typeof workspace === 'string')
    return { name: workspace || fallback, source: 'workspace' }
  if (typeof global === 'string') return { name: global || fallback, source: 'global' }
  return { name: fallback, source: 'builtin' }
}

export type AgentProfileMode = 'primary' | 'subagent' | 'all'
export type AgentIsolation = 'shared' | 'worktree'
export type ProjectInstructionsMode = 'startup' | 'on_demand'

export type AgentProfileContext = {
  project_instructions?: ProjectInstructionsMode
  preload_skills?: string[]
}

export type AgentProfileConfig = {
  title?: string
  description?: string
  extends?: string | null
  mode?: AgentProfileMode | null
  hidden?: boolean | null
  read_only?: boolean
  model?: JsonObject | null
  prompts?: Partial<PromptLayer>
  context?: AgentProfileContext
  isolation?: AgentIsolation
  permission?: JsonObject
  permission_profile?: string
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

/** Factory defaults survive a partial override; file scopes still replace named raw entries. */
export function withBuiltinAgentProfiles(
  builtins: AgentProfileCatalog,
  overrides: AgentProfileCatalog,
): AgentProfileCatalog {
  const catalog = agentProfileCatalog(builtins)
  for (const [name, overlay] of Object.entries(overrides)) {
    const base = catalog[name]
    if (!base) {
      catalog[name] = agentProfileCatalog({ [name]: overlay })[name]!
      continue
    }
    const next = { ...base }
    if (overlay.title) next.title = overlay.title
    if (overlay.description) next.description = overlay.description
    for (const key of ['mode', 'hidden', 'read_only', 'extends', 'model', 'max_turns', 'isolation'] as const) {
      if (overlay[key] != null) Object.assign(next, { [key]: overlay[key] })
    }
    if (overlay.context) {
      next.context = { ...base.context }
      const { project_instructions, preload_skills } = overlay.context
      if (project_instructions != null) next.context.project_instructions = project_instructions
      if (preload_skills != null) next.context.preload_skills = preload_skills.slice()
    }
    if (overlay.permission_profile) {
      next.permission_profile = overlay.permission_profile
      next.permission = overlay.permission
    } else if (overlay.permission)
      next.permission = mergeAgentProfilePermission(base.permission || {}, overlay.permission)
    if (overlay.prompts) {
      const layer = normalizePromptLayer(overlay.prompts)
      if (layer.mode === 'replace') next.prompts = layer
      else {
        const previous = normalizePromptLayer(base.prompts)
        const suppressRole = previous.inline.some(({ id }) => layer.disabled.includes(id))
        previous.order = previous.order.filter(
          (id) => !layer.disabled.includes(id) && !(suppressRole && id.startsWith('agena.profile.')),
        )
        previous.inline = previous.inline.filter((fragment) => !layer.disabled.includes(fragment.id))
        previous.disabled = [...new Set([...previous.disabled, ...layer.disabled])]
        for (const id of layer.order) previous.order = [...previous.order.filter((old) => old !== id), id]
        for (const fragment of layer.inline)
          previous.inline = [...previous.inline.filter((old) => old.id !== fragment.id), fragment]
        next.prompts = previous
      }
    }
    catalog[name] = next
  }
  return catalog
}

/** Merge profile permissions with the runtime's per-field and per-rule precedence. */
export function mergeAgentProfilePermission(base: JsonObject, overlay: JsonObject): JsonObject {
  const object = (value: unknown): JsonObject =>
    value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : {}
  function normalize(permission: JsonObject): JsonObject {
    try {
      return parsePermissionConfig(permission)
    } catch {
      return permission
    }
  }
  const lower = normalize(base)
  const upper = normalize(overlay)
  const result = { ...lower }
  if (upper.approval_model != null) result.approval_model = upper.approval_model
  if (upper.network != null) {
    try {
      result.network = mergeNetworkPermissionConfig(lower.network, upper.network)
    } catch {
      result.network = upper.network
    }
  }
  for (const section of ['path', 'tools'] as const) {
    if (upper[section] == null) continue
    const previous = object(lower[section])
    const patch = object(upper[section])
    const next = { ...previous, ...Object.fromEntries(Object.entries(patch).filter(([, value]) => value != null)) }
    for (const key of section === 'path' ? ['rules'] : ['names', 'plugin', 'rules']) {
      if (patch[key] == null) continue
      const parent = object(previous[key])
      const local = object(patch[key])
      next[key] =
        section === 'tools' && key !== 'plugin'
          ? Object.fromEntries([...canonicalPermissionToolEntries(parent), ...canonicalPermissionToolEntries(local)])
          : { ...parent, ...local }
    }
    result[section] = next
  }
  return result
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

/** Resolve presentation and selection metadata using the same parent-first rules as the server. */
export function agentProfileMetadata(
  catalog: AgentProfileCatalog,
  name: string,
): { mode: AgentProfileMode; hidden: boolean; readOnly: boolean } {
  let mode: AgentProfileMode = 'all'
  let hidden = false
  let readOnly = false
  for (const id of agentProfileLineage(catalog, name)) {
    const profile = catalog[id]!
    if (profile.mode != null) {
      if (!['primary', 'subagent', 'all'].includes(profile.mode))
        throw new Error(`Invalid profile mode: ${profile.mode}`)
      mode = profile.mode
    }
    if (profile.hidden != null) {
      if (typeof profile.hidden !== 'boolean') throw new Error('Profile hidden must be a boolean')
      hidden = profile.hidden
    }
    if (profile.read_only != null) {
      if (typeof profile.read_only !== 'boolean') throw new Error('Profile read_only must be a boolean')
      readOnly = profile.read_only
    }
  }
  return { mode, hidden, readOnly }
}

/** Hidden affects model discovery, while people can select every compatible role. */
export function selectableAgentProfiles(catalog: AgentProfileCatalog, subagent: boolean): AgentProfileCatalog {
  return Object.fromEntries(
    Object.entries(catalog).filter(([id]) => {
      try {
        const { mode } = agentProfileMetadata(catalog, id)
        return mode === 'all' || mode === (subagent ? 'subagent' : 'primary')
      } catch {
        return false
      }
    }),
  )
}
