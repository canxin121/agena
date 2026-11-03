export type PromptLayerMode = 'append' | 'replace'

export type PromptCondition = {
  models: string[]
  requires_tools: boolean | null
}

export type PromptFragment = {
  id: string
  title: string
  body: string
  when: PromptCondition
}

export type PromptLayer = {
  mode: PromptLayerMode
  order: string[]
  disabled: string[]
  inline: PromptFragment[]
}

export type ModelPromptLayer = PromptLayer & { match: string }

/** The global/workspace config value. `models` route overlays must survive edits. */
export type PromptScopeConfig = PromptLayer & { models: ModelPromptLayer[] }

export type PromptLayerScope = 'effective' | 'global' | 'workspace' | 'session'

export type PromptResolutionFragment = {
  id: string
  title: string
  scope: 'global' | 'workspace' | 'model' | 'session' | 'request'
  position: number
  bytes: number
  sha256: string
  text: string
}

export type PromptResolutionWarning = {
  scope: string
  id: string
  kind: 'condition_not_matched' | 'unknown_variable'
  detail: string
}

export type PromptResolution = {
  text: string
  fragments: PromptResolutionFragment[]
  warnings: PromptResolutionWarning[]
  total_bytes: number
  over_budget: boolean
}

export type PromptIssueCode =
  | 'invalid_id'
  | 'duplicate_order'
  | 'duplicate_inline'
  | 'duplicate_disabled'
  | 'order_disabled_conflict'
  | 'unknown_inline_fragment'
  | 'external_order_fragment'
  | 'inline_not_in_order'
  | 'fragment_too_large'
  | 'empty_body'

export type PromptIssue = {
  code: PromptIssueCode
  level: 'error' | 'warning'
  id: string
}

export const MAX_PROMPT_FRAGMENT_BYTES = 16 * 1024
export const MAX_PROMPT_TOTAL_BYTES = 128 * 1024
// Mirrors FragmentId::parse in agena-prompt-layers: 1-64 ASCII characters,
// lowercase letters, digits, '.', '_' or '-', never starting with a dot.
const FRAGMENT_ID_PATTERN = /^(?!\.)[a-z0-9._-]{1,64}$/

function record(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
}

function text(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function nullableBoolean(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

export function emptyPromptCondition(): PromptCondition {
  return { models: [], requires_tools: null }
}

export function normalizePromptCondition(value: unknown): PromptCondition {
  const item = record(value)
  return {
    models: strings(item.models)
      .map((model) => model.trim())
      .filter(Boolean),
    requires_tools: nullableBoolean(item.requires_tools),
  }
}

export function normalizePromptFragment(value: unknown): PromptFragment {
  const item = record(value)
  return {
    id: text(item.id),
    title: text(item.title),
    body: text(item.body),
    when: normalizePromptCondition(item.when),
  }
}

export function emptyPromptLayer(): PromptLayer {
  return { mode: 'append', order: [], disabled: [], inline: [] }
}

export function normalizePromptLayer(value: unknown): PromptLayer {
  const item = record(value)
  return {
    mode: item.mode === 'replace' ? 'replace' : 'append',
    order: strings(item.order),
    disabled: strings(item.disabled),
    inline: Array.isArray(item.inline) ? item.inline.map(normalizePromptFragment) : [],
  }
}

export function normalizeModelPromptLayer(value: unknown): ModelPromptLayer {
  const item = record(value)
  return { match: text(item.match), ...normalizePromptLayer(item) }
}

export function normalizePromptScopeConfig(value: unknown): PromptScopeConfig {
  const item = record(value)
  return {
    ...normalizePromptLayer(item),
    models: Array.isArray(item.models) ? item.models.map(normalizeModelPromptLayer) : [],
  }
}

export function emptyPromptScopeConfig(): PromptScopeConfig {
  return { ...emptyPromptLayer(), models: [] }
}

export function clonePromptLayer(value: PromptLayer): PromptLayer {
  return normalizePromptLayer(value)
}

export function clonePromptScopeConfig(value: PromptScopeConfig): PromptScopeConfig {
  return normalizePromptScopeConfig(value)
}

export function promptLayerFromScopeConfig(value: PromptScopeConfig): PromptLayer {
  return normalizePromptLayer(value)
}

export function promptScopeConfigWithLayer(value: PromptScopeConfig, layer: PromptLayer): PromptScopeConfig {
  return { ...normalizePromptLayer(layer), models: value.models.map(normalizeModelPromptLayer) }
}

export function promptLayerIsEmpty(value: PromptLayer): boolean {
  return value.mode === 'append' && !value.order.length && !value.disabled.length && !value.inline.length
}

export function promptScopeConfigIsEmpty(value: PromptScopeConfig): boolean {
  return promptLayerIsEmpty(value) && !value.models.length
}

export function promptLayersEqual(left: PromptLayer, right: PromptLayer): boolean {
  return JSON.stringify(normalizePromptLayer(left)) === JSON.stringify(normalizePromptLayer(right))
}

export function promptScopeConfigsEqual(left: PromptScopeConfig, right: PromptScopeConfig): boolean {
  return JSON.stringify(normalizePromptScopeConfig(left)) === JSON.stringify(normalizePromptScopeConfig(right))
}

export function promptFragmentBodyBytes(body: string): number {
  return new TextEncoder().encode(body).length
}

export function isValidPromptFragmentId(id: string): boolean {
  return FRAGMENT_ID_PATTERN.test(id)
}

/** Mirrors the Rust resolver's structural checks before a round trip. */
export function validatePromptLayer(layer: PromptLayer, allowExternalFragments = false): PromptIssue[] {
  const issues: PromptIssue[] = []
  const inlineIds = new Set<string>()
  const orderIds = new Set<string>()
  const disabledIds = new Set<string>()

  for (const fragment of layer.inline) {
    if (!isValidPromptFragmentId(fragment.id)) issues.push({ code: 'invalid_id', level: 'error', id: fragment.id })
    if (inlineIds.has(fragment.id)) issues.push({ code: 'duplicate_inline', level: 'error', id: fragment.id })
    inlineIds.add(fragment.id)
    if (promptFragmentBodyBytes(fragment.body) > MAX_PROMPT_FRAGMENT_BYTES) {
      issues.push({ code: 'fragment_too_large', level: 'error', id: fragment.id })
    }
    if (!fragment.body.trim()) issues.push({ code: 'empty_body', level: 'warning', id: fragment.id })
  }
  for (const id of layer.order) {
    if (!isValidPromptFragmentId(id)) issues.push({ code: 'invalid_id', level: 'error', id })
    if (orderIds.has(id)) issues.push({ code: 'duplicate_order', level: 'error', id })
    orderIds.add(id)
  }
  for (const id of layer.disabled) {
    if (!isValidPromptFragmentId(id)) issues.push({ code: 'invalid_id', level: 'error', id })
    // Duplicate entries have no effect on resolution, so the server accepts
    // them. Surface them as a warning instead of blocking a save.
    if (disabledIds.has(id)) issues.push({ code: 'duplicate_disabled', level: 'warning', id })
    disabledIds.add(id)
  }
  for (const id of orderIds) {
    if (!inlineIds.has(id)) {
      issues.push({
        code: allowExternalFragments ? 'external_order_fragment' : 'unknown_inline_fragment',
        level: allowExternalFragments ? 'warning' : 'error',
        id,
      })
    }
  }
  // Rust rejects `disabled` entries that are also ordered or defined inline.
  for (const id of new Set([...orderIds, ...inlineIds])) {
    if (disabledIds.has(id)) issues.push({ code: 'order_disabled_conflict', level: 'error', id })
  }
  for (const id of inlineIds) {
    if (!orderIds.has(id)) issues.push({ code: 'inline_not_in_order', level: 'warning', id })
  }
  return issues
}

export function promptLayerHasErrors(issues: readonly PromptIssue[]): boolean {
  return issues.some((issue) => issue.level === 'error')
}

export function promptLayerAddFragment(layer: PromptLayer, fragment: PromptFragment): PromptLayer {
  const next = clonePromptLayer(layer)
  if (next.inline.some((entry) => entry.id === fragment.id)) return next
  next.inline.push(normalizePromptFragment(fragment))
  // An external library fragment may already occupy this id in `order`; the
  // inline definition then shadows it without introducing a duplicate.
  if (!next.order.includes(fragment.id)) next.order.push(fragment.id)
  next.disabled = next.disabled.filter((id) => id !== fragment.id)
  return next
}

export function promptLayerUpdateFragment(layer: PromptLayer, index: number, fragment: PromptFragment): PromptLayer {
  const next = clonePromptLayer(layer)
  const previous = next.inline[index]
  if (!previous) return next
  const updated = normalizePromptFragment(fragment)
  if (previous.id !== updated.id) {
    next.order = next.order.map((id) => (id === previous.id ? updated.id : id))
    next.disabled = next.disabled.map((id) => (id === previous.id ? updated.id : id))
  }
  next.inline[index] = updated
  return next
}

export function promptLayerRemoveFragment(layer: PromptLayer, id: string): PromptLayer {
  return {
    ...clonePromptLayer(layer),
    inline: layer.inline.filter((fragment) => fragment.id !== id).map(normalizePromptFragment),
    order: layer.order.filter((entry) => entry !== id),
    disabled: layer.disabled.filter((entry) => entry !== id),
  }
}

function modelLayerOwnsFragment(model: ModelPromptLayer, id: string): boolean {
  return model.inline.some((fragment) => fragment.id === id)
}

function renameModelLayerReferences(model: ModelPromptLayer, oldId: string, newId: string): ModelPromptLayer {
  if (modelLayerOwnsFragment(model, oldId)) return model
  return {
    ...model,
    order: model.order.map((id) => (id === oldId ? newId : id)),
    disabled: model.disabled.map((id) => (id === oldId ? newId : id)),
  }
}

function removeModelLayerReferences(model: ModelPromptLayer, id: string): ModelPromptLayer {
  if (modelLayerOwnsFragment(model, id)) return model
  return {
    ...model,
    order: model.order.filter((entry) => entry !== id),
    disabled: model.disabled.filter((entry) => entry !== id),
  }
}

/** Update a top-level fragment and keep model-layer references to it in sync. */
export function promptScopeConfigUpdateFragment(
  value: PromptScopeConfig,
  index: number,
  fragment: PromptFragment,
): PromptScopeConfig {
  const previous = value.inline[index]
  const next = promptScopeConfigWithLayer(value, promptLayerUpdateFragment(value, index, fragment))
  const updated = next.inline[index]
  if (!previous || !updated || previous.id === updated.id) return next
  return {
    ...next,
    models: next.models.map((model) => renameModelLayerReferences(model, previous.id, updated.id)),
  }
}

/** Remove a top-level fragment and keep model-layer references to it in sync. */
export function promptScopeConfigRemoveFragment(value: PromptScopeConfig, id: string): PromptScopeConfig {
  const next = promptScopeConfigWithLayer(value, promptLayerRemoveFragment(value, id))
  return {
    ...next,
    models: next.models.map((model) => removeModelLayerReferences(model, id)),
  }
}

export function promptLayerRemoveOrder(layer: PromptLayer, id: string): PromptLayer {
  return { ...clonePromptLayer(layer), order: layer.order.filter((entry) => entry !== id) }
}

export function promptLayerAddOrder(layer: PromptLayer, id: string): PromptLayer {
  const next = clonePromptLayer(layer)
  if (!next.order.includes(id)) next.order.push(id)
  next.disabled = next.disabled.filter((entry) => entry !== id)
  return next
}

export function promptLayerMoveOrder(layer: PromptLayer, id: string, delta: number): PromptLayer {
  const next = clonePromptLayer(layer)
  const index = next.order.indexOf(id)
  if (index < 0) return next
  const target = Math.max(0, Math.min(next.order.length - 1, index + delta))
  if (target === index) return next
  const [entry] = next.order.splice(index, 1)
  next.order.splice(target, 0, entry)
  return next
}

export function promptLayerSetDisabled(layer: PromptLayer, id: string, disabled: boolean): PromptLayer {
  const next = clonePromptLayer(layer)
  if (disabled) {
    next.order = next.order.filter((entry) => entry !== id)
    // Inline fragments are disabled by omitting them from `order`; a
    // `disabled` entry is only for fragments inherited from broader scopes.
    if (!next.inline.some((fragment) => fragment.id === id) && !next.disabled.includes(id)) {
      next.disabled.push(id)
    }
  } else {
    next.disabled = next.disabled.filter((entry) => entry !== id)
  }
  return next
}

export function nextPromptFragmentId(layer: PromptLayer): string {
  for (let index = 1; index < 10_000; index += 1) {
    const candidate = `fragment-${index}`
    if (
      !layer.inline.some((fragment) => fragment.id === candidate) &&
      !layer.order.includes(candidate) &&
      !layer.disabled.includes(candidate)
    ) {
      return candidate
    }
  }
  return 'fragment-extra'
}

export function promptFragmentConditionSummary(fragment: PromptFragment): string {
  const parts: string[] = []
  if (fragment.when.models.length) parts.push(fragment.when.models.join(', '))
  if (fragment.when.requires_tools === true) parts.push('tools')
  if (fragment.when.requires_tools === false) parts.push('no tools')
  return parts.join(' · ')
}
