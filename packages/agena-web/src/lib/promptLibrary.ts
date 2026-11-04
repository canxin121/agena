import {
  isValidPromptFragmentId,
  normalizePromptFragment,
  type PromptFragment,
  type PromptLayerScope,
  type PromptScopeConfig,
} from './promptLayers'

export type SavedPromptOption = {
  id: string
  fragment: PromptFragment
  scope: 'global' | 'workspace'
  legacy: boolean
}

/**
 * The same name/id in the workspace shadows the global one in sessions.
 * A global layer cannot reference workspace prompts.
 * Legacy inline prompts are listed for compatibility with existing installs.
 */
export function availableSavedPrompts(
  global: PromptScopeConfig,
  workspace: PromptScopeConfig,
  target: PromptLayerScope,
): SavedPromptOption[] {
  const items = new Map<string, SavedPromptOption>()
  const add = (source: PromptScopeConfig, scope: 'global' | 'workspace') => {
    for (const fragment of source.library) {
      items.set(fragment.id, { id: fragment.id, fragment: normalizePromptFragment(fragment), scope, legacy: false })
    }
    for (const fragment of source.inline) {
      items.set(fragment.id, { id: fragment.id, fragment: normalizePromptFragment(fragment), scope, legacy: true })
    }
  }
  add(global, 'global')
  if (target !== 'global') add(workspace, 'workspace')
  return [...items.values()].sort((a, b) =>
    (a.fragment.title || a.id).localeCompare(b.fragment.title || b.id),
  )
}

export function nextSavedPromptId(
  scope: 'global' | 'workspace',
  global: PromptScopeConfig,
  workspace: PromptScopeConfig,
): string {
  const used = new Set<string>()
  for (const config of [global, workspace]) {
    for (const fragment of [...config.library, ...config.inline]) used.add(fragment.id)
    for (const id of [...config.order, ...config.disabled]) used.add(id)
    for (const model of config.models) {
      for (const fragment of model.inline) used.add(fragment.id)
      for (const id of [...model.order, ...model.disabled]) used.add(id)
    }
  }
  for (let index = 1; index < 1_000_000; index++) {
    const id = `saved-${scope}-${index}`
    if (!used.has(id)) return id
  }
  throw new Error('No available saved prompt identifier')
}

export function savedPromptDisplayName(fragment: PromptFragment): string {
  return fragment.title.trim() || fragment.id
}

export function savedPromptMatches(fragment: PromptFragment, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase()
  return !needle || [fragment.title, fragment.category || '', fragment.body, fragment.id].some((part) =>
    part.toLocaleLowerCase().includes(needle),
  )
}

export function savedPromptIsValid(fragment: PromptFragment): boolean {
  return (
    isValidPromptFragmentId(fragment.id) &&
    fragment.title.trim().length > 0 &&
    fragment.body.trim().length > 0 &&
    new TextEncoder().encode(fragment.body).length <= 16 * 1024
  )
}

export function scopeUsesPrompt(config: PromptScopeConfig, id: string): boolean {
  return config.order.includes(id) || config.models.some((model) => model.order.includes(id))
}
