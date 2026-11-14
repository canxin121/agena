import type { PluginSettingsTab } from '@/components/settings/pluginSettingsNavigation'
import { TELEGRAM_PAGE_ID, TELEGRAM_PLUGIN_ID } from './hub'
import { pluginPagePath, pluginPageSession } from './model'
import type { PluginPageCatalogItem } from './sdk'

/** A launch action in the existing plugin settings, not another plugin catalog. */
export type PluginInteractionPage = Pick<PluginPageCatalogItem, 'plugin_id' | 'page_id' | 'title' | 'scope'> & {
  native?: boolean
}

const settingsTabs = new Set<PluginSettingsTab>([
  'overview',
  'settings',
  'tools',
  'commands',
  'hooks',
  'logs',
  'diagnostics',
])
export function pluginReturnTab(raw: unknown): PluginSettingsTab {
  return typeof raw === 'string' && settingsTabs.has(raw as PluginSettingsTab) ? (raw as PluginSettingsTab) : 'overview'
}

export function pluginSettingsLocation(pluginId = '', rawTab: unknown = 'overview') {
  const tab = pluginReturnTab(rawTab)
  const query: Record<string, string> = { view: 'plugin-workbench' }
  if (pluginId) query.plugin = pluginId
  if (pluginId && tab !== 'overview') query.pluginTab = tab
  return { path: '/settings/plugins-tools', query }
}

export function pluginInteractionPages(pluginId: string, pages: PluginPageCatalogItem[]): PluginInteractionPage[] {
  if (!pluginId) return []
  const selected: PluginInteractionPage[] = pages
    .filter((page) => page.plugin_id === pluginId)
    .map(({ plugin_id, page_id, title, scope }) => ({ plugin_id, page_id, title, scope }))
  // Telegram is a host-provided application for the existing bundled plugin;
  // it has no external surface.web declaration. Deduplicate by stable page ID.
  if (pluginId === TELEGRAM_PLUGIN_ID && !selected.some((page) => page.page_id === TELEGRAM_PAGE_ID)) {
    selected.unshift({
      plugin_id: pluginId,
      page_id: TELEGRAM_PAGE_ID,
      title: 'Telegram',
      scope: 'instance',
      native: true,
    })
  }
  return selected
}

export function pluginInteractionLocation(page: PluginInteractionPage, tab: unknown, selectedSession?: unknown) {
  const query: Record<string, string> = { settingsTab: pluginReturnTab(tab) }
  const sessionId = pluginPageSession(selectedSession)
  if (page.scope === 'session' && sessionId !== null) query.sessionId = String(sessionId)
  return { path: pluginPagePath(page.plugin_id, page.page_id), query }
}
