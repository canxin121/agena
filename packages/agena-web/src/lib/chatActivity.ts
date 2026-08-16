import type { JsonValue } from '@/types/json'

export type ChatActivityKindCategory = 'builtin' | 'plugin'

export type ChatActivityKindCatalogItem = {
  id: string
  category: ChatActivityKindCategory
  label: string
}

// Mirrors agena_domain::builtin_activity_kinds. The server response remains
// authoritative and can append plugin-contributed kinds at runtime.
export const BUILTIN_CHAT_ACTIVITY_KINDS: ChatActivityKindCatalogItem[] = [
  { id: 'reasoning', category: 'builtin', label: 'Reasoning' },
  { id: 'operation', category: 'builtin', label: 'Operation' },
  { id: 'resource', category: 'builtin', label: 'Resource' },
  { id: 'skill_reference', category: 'builtin', label: 'Skill reference' },
  { id: 'interaction', category: 'builtin', label: 'Interaction' },
  { id: 'hook', category: 'builtin', label: 'Hook' },
  { id: 'error', category: 'builtin', label: 'Error' },
  { id: 'notice', category: 'builtin', label: 'Notice' },
  { id: 'text', category: 'builtin', label: 'Text' },
]

export const DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED = ['reasoning']

const LEGACY_ACTIVITY_KIND_ALIASES: Record<string, string> = {
  thinking: 'reasoning',
  compaction: 'notice',
}

function normalizedStringList(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  const out: string[] = []
  const seen = new Set<string>()
  for (const item of value) {
    if (typeof item !== 'string') continue
    const id = item.trim()
    if (!id || seen.has(id)) continue
    seen.add(id)
    out.push(id)
  }
  return out
}

export function normalizeChatActivityKindCatalog(value: unknown): ChatActivityKindCatalogItem[] {
  if (!Array.isArray(value)) return []
  const out: ChatActivityKindCatalogItem[] = []
  const seen = new Set<string>()
  for (const raw of value) {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) continue
    const item = raw as Record<string, unknown>
    const id = typeof item.id === 'string' ? item.id.trim() : ''
    if (!id || seen.has(id)) continue
    seen.add(id)
    out.push({
      id,
      category: item.category === 'plugin' ? 'plugin' : 'builtin',
      label: typeof item.label === 'string' && item.label.trim() ? item.label.trim() : id,
    })
  }
  return out
}

export function normalizeChatActivityKindDefaultExpanded(value: unknown): string[] {
  return normalizedStringList(value)
}

export function migrateLegacyChatActivityDefaultExpanded(value: unknown): string[] {
  const expanded = new Set(DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED)
  const builtinIds = new Set(BUILTIN_CHAT_ACTIVITY_KINDS.map((item) => item.id))
  for (const legacyId of normalizedStringList(value)) {
    const normalizedLegacyId = legacyId.toLowerCase()
    const id = LEGACY_ACTIVITY_KIND_ALIASES[normalizedLegacyId] || normalizedLegacyId
    if (builtinIds.has(id)) expanded.add(id)
  }
  return [...expanded]
}

export function resolveChatActivityKindDefaultExpanded(settings: unknown): string[] {
  if (!settings || typeof settings !== 'object' || Array.isArray(settings)) {
    return DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED.slice()
  }
  const record = settings as Record<string, unknown>
  if (Object.prototype.hasOwnProperty.call(record, 'chatActivityKindDefaultExpanded')) {
    return normalizeChatActivityKindDefaultExpanded(record.chatActivityKindDefaultExpanded)
  }
  if (Object.prototype.hasOwnProperty.call(record, 'chatActivityDefaultExpanded')) {
    return migrateLegacyChatActivityDefaultExpanded(record.chatActivityDefaultExpanded)
  }
  return DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED.slice()
}

export function chatActivityKindIdForTranscriptPart(partKind: unknown, durablePartKind: unknown): string {
  const presentation = typeof partKind === 'string' ? partKind.trim().toLowerCase() : ''
  const durableId = typeof durablePartKind === 'string' ? durablePartKind.trim() : ''
  const durable = durableId.toLowerCase()
  if (presentation === 'reasoning') return 'reasoning'
  if (presentation === 'operation') return 'operation'
  if (presentation === 'resource') return 'resource'
  if (presentation === 'skill') return 'skill_reference'
  if (presentation === 'interaction') return 'interaction'
  if (presentation === 'error') return 'error'
  if (presentation === 'text_segment') return 'text'
  if (presentation === 'notice') return durable === 'hook' ? 'hook' : 'notice'
  if (presentation === 'compaction') return 'notice'
  if (presentation === 'unknown') return durableId
  return ''
}

export type KnownChatToolActivityType =
  | 'read'
  | 'list'
  | 'glob'
  | 'grep'
  | 'edit'
  | 'write'
  | 'apply_patch'
  | 'multiedit'
  | 'bash'
  | 'task'
  | 'webfetch'
  | 'websearch'
  | 'codesearch'
  | 'skill'
  | 'lsp'
  | 'todowrite'
  | 'todoread'
  | 'question'
  | 'batch'
  | 'plan_enter'
  | 'plan_exit'
  | 'unknown'

export type ChatToolActivityType = KnownChatToolActivityType | (string & {})
export type ChatToolExpansionOverrides = Record<string, boolean>

export const DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS: ChatToolActivityType[] = [
  'read',
  'list',
  'glob',
  'grep',
  'edit',
  'write',
  'apply_patch',
  'multiedit',
  'bash',
  'task',
  'webfetch',
  'websearch',
  'codesearch',
  'skill',
  'lsp',
  'todowrite',
  'todoread',
  'question',
  'batch',
  'plan_enter',
  'plan_exit',
  'unknown',
]

export const DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS: ChatToolActivityType[] = [
  'edit',
  'write',
  'apply_patch',
  'multiedit',
]

const CHAT_TOOL_ACTIVITY_SET = new Set(DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS)

export function isKnownChatToolActivityType(value: string): value is KnownChatToolActivityType {
  return CHAT_TOOL_ACTIVITY_SET.has(value as KnownChatToolActivityType)
}

export function normalizeChatToolActivityId(value: unknown): string {
  const raw = typeof value === 'string' ? value.trim().toLowerCase() : ''
  if (!raw || isKnownChatToolActivityType(raw)) return raw

  const [namespace = '', ...nameParts] = raw.split('.')
  const name = nameParts.join('.')
  if (namespace === 'fs') {
    if (['read', 'read_many', 'stat', 'view_image'].includes(name)) return 'read'
    if (name === 'write') return 'write'
    if (name === 'replace') return 'edit'
    if (name === 'apply_patch') return 'apply_patch'
    if (name === 'glob') return 'glob'
    if (name === 'grep') return 'grep'
  }
  if (namespace === 'shell') return 'bash'
  if (namespace === 'web') return name === 'search' ? 'websearch' : 'webfetch'
  if (namespace === 'code') return 'codesearch'
  if (namespace === 'interaction' && name === 'ask') return 'question'
  if (namespace === 'tasks') return 'task'
  if (namespace === 'skills') return 'skill'
  if (namespace === 'lsp') return 'lsp'

  if (['chatgpt', 'claude', 'gemini'].includes(namespace)) {
    if (['bash', 'shell', 'local_shell', 'code_execution'].includes(name)) return 'bash'
    if (name === 'apply_patch') return 'apply_patch'
    if (name === 'text_editor') return 'edit'
    if (['web_fetch', 'url_context'].includes(name)) return 'webfetch'
    if (['web_search', 'web_search_preview', 'google_search'].includes(name)) return 'websearch'
    if (['file_search', 'tool_search', 'tool_search_bm25', 'tool_search_regex'].includes(name)) return 'codesearch'
  }

  return raw
}

/**
 * Stable exact identity used by per-tool presentation preferences.
 *
 * Runtime parts commonly use `fs.read` while the plugin catalog advertises
 * `agena.fs.read`. The leading Agena registry namespace is transport metadata,
 * not a different tool, so both shapes share one preference key. Unlike
 * `normalizeChatToolActivityId`, this function never folds tools into broad
 * categories such as `read` or `bash`.
 */
export function normalizeChatToolPreferenceId(value: unknown): string {
  const raw = typeof value === 'string' ? value.trim().toLowerCase() : ''
  return raw.startsWith('agena.') ? raw.slice('agena.'.length) : raw
}

export function normalizeChatToolExpansionOverrides(value: unknown): ChatToolExpansionOverrides {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  const out: ChatToolExpansionOverrides = {}
  for (const [rawTool, rawExpanded] of Object.entries(value as Record<string, unknown>)) {
    const tool = normalizeChatToolPreferenceId(rawTool)
    if (!tool || typeof rawExpanded !== 'boolean') continue
    out[tool] = rawExpanded
  }
  return out
}

export function resolveChatToolDefaultExpanded(
  toolName: unknown,
  overrides: ChatToolExpansionOverrides,
  legacyExpandedCategories: ReadonlySet<string>,
  operationDefaultExpanded = false,
): boolean {
  const exactTool = normalizeChatToolPreferenceId(toolName)
  if (exactTool && Object.prototype.hasOwnProperty.call(overrides, exactTool)) {
    return overrides[exactTool] === true
  }

  const category = normalizeChatToolActivityId(exactTool)
  if (!category) return legacyExpandedCategories.has('unknown')
  if (legacyExpandedCategories.has(category)) return true
  if (isKnownChatToolActivityType(category)) return operationDefaultExpanded
  return legacyExpandedCategories.has('unknown') || operationDefaultExpanded
}

export function normalizeChatToolActivityFilters(value: JsonValue): ChatToolActivityType[] {
  const out: ChatToolActivityType[] = []
  const seen = new Set<string>()
  if (Array.isArray(value)) {
    for (const item of value) {
      if (typeof item !== 'string') continue
      let key = normalizeChatToolActivityId(item)
      // Backward compatibility with older builds.
      if (key === 'invalid') key = 'unknown'
      if (!key) continue
      if (seen.has(key)) continue
      seen.add(key)
      out.push(key)
    }
  }
  return out
}
