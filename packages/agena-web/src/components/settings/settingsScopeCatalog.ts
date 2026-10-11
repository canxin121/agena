/**
 * Scope-aware settings pages read or write one configuration layer at a time:
 * the effective (merged) view, the global layer, the current workspace, or the
 * current session. The left navigation renders those layers as a third level
 * under the page row, so a page never needs its own scope picker.
 */

import type { SettingsTab } from './sidebar/settingsSidebarNavigation'
import { settingsText as st } from '../../i18n/settingsText'

export const SETTINGS_SCOPE_IDS = ['effective', 'global', 'workspace', 'session'] as const

export type SettingsScopeId = (typeof SETTINGS_SCOPE_IDS)[number]

export type SettingsScopeDefinition = {
  id: SettingsScopeId
  label: string
  description: string
}

type SettingsScopeSource = {
  id: SettingsScopeId
  label: () => string
  description: () => string
}

// Every permission page compares the same four layers, so they share one set.
const PERMISSION_SCOPE_SOURCES: readonly SettingsScopeSource[] = [
  {
    id: 'effective',
    label: () => st('Effective Permission'),
    description: () => st('Read-only merged policy.'),
  },
  {
    id: 'global',
    label: () => st('Global Permission'),
    description: () => st('Baseline for all sessions.'),
  },
  {
    id: 'workspace',
    label: () => st('Workspace Permission'),
    description: () => st('Overrides for this workspace.'),
  },
  {
    id: 'session',
    label: () => st('Current Session Permission'),
    description: () => st('Applies only to the selected session.'),
  },
]

// A session can run one model, but it cannot pin a default for later runs, so
// the model defaults only resolve three layers.
const MODEL_DEFAULT_SCOPE_SOURCES: readonly SettingsScopeSource[] = [
  {
    id: 'effective',
    label: () => st('Effective default'),
    description: () => st('Read-only merged default.'),
  },
  {
    id: 'global',
    label: () => st('Global default'),
    description: () => st('Baseline for every workspace.'),
  },
  {
    id: 'workspace',
    label: () => st('Workspace default'),
    description: () => st('Overrides the global default for this workspace.'),
  },
]

// The advanced editor writes one explicit layer; the effective value stays a
// read-only comparison section inside the page.
const LAYER_SCOPE_SOURCES: readonly SettingsScopeSource[] = [
  {
    id: 'global',
    label: () => st('Global layer'),
    description: () => st('Writes the server-wide Agena configuration file.'),
  },
  {
    id: 'workspace',
    label: () => st('Workspace layer'),
    description: () => st('Writes the current workspace configuration file.'),
  },
]

const SETTINGS_PAGE_SCOPE_SOURCES: Partial<Record<SettingsTab, Record<string, readonly SettingsScopeSource[]>>> = {
  permissions: {
    filesystem: PERMISSION_SCOPE_SOURCES,
    network: PERMISSION_SCOPE_SOURCES,
    tools: PERMISSION_SCOPE_SOURCES,
  },
  'models-providers': {
    defaults: MODEL_DEFAULT_SCOPE_SOURCES,
    'approval-model': MODEL_DEFAULT_SCOPE_SOURCES,
  },
  diagnostics: {
    'advanced-settings': LAYER_SCOPE_SOURCES,
  },
}

export function normalizeSettingsScopeId(value: unknown): string {
  return String(value || '')
    .trim()
    .toLowerCase()
}

function settingsScopeSources(section: SettingsTab | string, page: unknown): readonly SettingsScopeSource[] {
  const sources = SETTINGS_PAGE_SCOPE_SOURCES[section as SettingsTab]
  if (!sources) return []
  return sources[normalizeSettingsScopeId(page)] || []
}

export function buildSettingsScopes(section: SettingsTab | string, page: unknown): SettingsScopeDefinition[] {
  return settingsScopeSources(section, page).map((scope) => ({
    id: scope.id,
    label: scope.label(),
    description: scope.description(),
  }))
}

export function settingsScopeIdsForPage(section: SettingsTab | string, page: unknown): SettingsScopeId[] {
  return settingsScopeSources(section, page).map((scope) => scope.id)
}

export function findSettingsScope(
  section: SettingsTab | string,
  page: unknown,
  scope: unknown,
): SettingsScopeDefinition | null {
  const id = normalizeSettingsScopeId(scope)
  if (!id) return null
  return buildSettingsScopes(section, page).find((entry) => entry.id === id) || null
}

/** Scope memory is per section: a permission layer survives a page change inside the section. */
export function settingsScopeStorageKey(section: SettingsTab | string): string {
  const normalized = normalizeSettingsScopeId(section) || 'unknown'
  return `studio.settings.scope.${normalized}`
}

/**
 * A remembered scope is only reused while the active page still resolves it;
 * a page with fewer layers falls back to its own first scope instead of keeping
 * a layer it cannot show. Pages without a layer choice resolve to an empty scope.
 */
export function resolveSettingsScope(
  requested: unknown,
  remembered: unknown,
  scopes: readonly SettingsScopeDefinition[],
  fallback?: unknown,
): string {
  const ids = new Set<string>(scopes.map((scope) => scope.id))
  if (!ids.size) return ''
  for (const candidate of [requested, remembered, fallback, scopes[0]?.id]) {
    const normalized = normalizeSettingsScopeId(candidate)
    if (normalized && ids.has(normalized)) return normalized
  }
  return ''
}
