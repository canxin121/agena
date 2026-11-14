import type { PluginPageCatalogItem } from './sdk'

/** Runtime plugins and Web pages are independent surfaces: a plugin can exist
 * without contributing any browser module at all.
 */
export type PluginRuntimeStatus = {
  plugin_id: string
  kind: string
  state: string
  summary?: string | null
  translations?: Record<string, { summary?: string | null } | undefined>
  last_failure?: { message?: string; rendered?: string; user?: { fallback?: string } } | null
}

export type PluginHubEntry = {
  plugin_id: string
  title: string
  summary: string
  kind: string
  state: string
  pages: PluginPageCatalogItem[]
  native: boolean
}

export const TELEGRAM_PLUGIN_ID = 'agena.telegram'
export const TELEGRAM_PAGE_ID = 'manage'

export function parsePluginStatusList(value: unknown): PluginRuntimeStatus[] {
  const items = Array.isArray(value)
    ? value
    : value && typeof value === 'object' && 'items' in value
      ? (value as { items: unknown }).items
      : null
  if (!Array.isArray(items)) throw new Error('Invalid plugin status catalog')
  const keys = new Set<string>()
  return items.map((item) => {
    if (!item || typeof item !== 'object') throw new Error('Invalid plugin status')
    const status = item as PluginRuntimeStatus
    if (
      typeof status.plugin_id !== 'string' ||
      !status.plugin_id.trim() ||
      typeof status.state !== 'string' ||
      typeof status.kind !== 'string'
    )
      throw new Error('Incomplete plugin status')
    if (keys.has(status.plugin_id)) throw new Error('Duplicate plugin status')
    keys.add(status.plugin_id)
    return status
  })
}

export function pluginHubEntries(
  statuses: PluginRuntimeStatus[],
  pages: PluginPageCatalogItem[],
  locale: string,
): PluginHubEntry[] {
  const entries = new Map<string, PluginHubEntry>()
  for (const status of statuses) {
    const summary =
      status.translations?.[locale]?.summary ||
      status.translations?.[locale.split('-')[0] || '']?.summary ||
      status.summary ||
      ''
    entries.set(status.plugin_id, {
      plugin_id: status.plugin_id,
      title: status.plugin_id === TELEGRAM_PLUGIN_ID ? 'Telegram' : status.plugin_id,
      summary,
      kind: status.kind,
      state: status.state,
      pages: [],
      native: status.plugin_id === TELEGRAM_PLUGIN_ID,
    })
  }
  for (const page of pages) {
    let entry = entries.get(page.plugin_id)
    if (!entry) {
      entry = {
        plugin_id: page.plugin_id,
        title: page.plugin_id,
        summary: '',
        kind: '',
        state: 'unknown',
        pages: [],
        native: page.plugin_id === TELEGRAM_PLUGIN_ID,
      }
      entries.set(page.plugin_id, entry)
    }
    entry.pages.push(page)
  }
  for (const entry of entries.values()) entry.pages.sort((a, b) => a.title.localeCompare(b.title, locale))
  return [...entries.values()].sort(
    (a, b) => Number(b.native) - Number(a.native) || a.title.localeCompare(b.title, locale),
  )
}
