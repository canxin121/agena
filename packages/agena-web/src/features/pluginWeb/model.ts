import type { PluginPageCatalogItem } from './sdk'

export function pluginPagePath(pluginId: string, pageId: string): string {
  return `/plugins/${encodeURIComponent(pluginId)}/${encodeURIComponent(pageId)}`
}

export function pluginPageKey(page: Pick<PluginPageCatalogItem, 'plugin_id' | 'page_id'>): string {
  return JSON.stringify([page.plugin_id, page.page_id])
}

/** Asset locations originate in the server catalog, and must stay in this exact
 * plugin revision. Normalize with URL before checking to reject encoded traversal.
 */
export function pluginAssetUrl(page: PluginPageCatalogItem, path: string, origin: string): string {
  const base = new URL(origin)
  const url = new URL(path, base)
  const prefix = `/plugin-assets/${encodeURIComponent(page.plugin_id)}/${encodeURIComponent(page.revision)}/`
  if (
    !['http:', 'https:'].includes(url.protocol) ||
    url.origin !== base.origin ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    path.includes('\\') ||
    !url.pathname.startsWith(prefix) ||
    url.pathname === prefix
  )
    throw new Error('Invalid plugin asset URL')
  const relative = url.pathname.slice(prefix.length)
  for (const segment of relative.split('/')) {
    const value = decodeURIComponent(segment)
    if (!value || value === '.' || value === '..' || /[\\/%\u0000-\u001f\u007f]/.test(value))
      throw new Error('Invalid plugin asset path')
  }
  return url.href
}

export function parsePluginPages(value: unknown, origin: string): PluginPageCatalogItem[] {
  if (value === undefined) return []
  if (!Array.isArray(value)) throw new Error('Invalid plugin page catalog')
  const keys = new Set<string>()
  return value.map((raw) => {
    if (!raw || typeof raw !== 'object') throw new Error('Invalid plugin page')
    const page = raw as PluginPageCatalogItem
    for (const field of ['plugin_id', 'page_id', 'title', 'entry_url', 'revision', 'generation'] as const) {
      if (typeof page[field] !== 'string' || !page[field].trim()) throw new Error('Incomplete plugin page')
    }
    if (
      page.api_version !== 1 ||
      !['instance', 'session'].includes(page.scope) ||
      typeof page.settings !== 'boolean' ||
      !Array.isArray(page.commands) ||
      !page.commands.every((command) => typeof command === 'string' && command.length > 0) ||
      !Array.isArray(page.style_urls) ||
      !page.style_urls.every((style) => typeof style === 'string')
    )
      throw new Error('Unsupported plugin page')
    pluginAssetUrl(page, page.entry_url, origin)
    for (const style of page.style_urls) pluginAssetUrl(page, style, origin)
    const key = pluginPageKey(page)
    if (keys.has(key)) throw new Error('Duplicate plugin page')
    keys.add(key)
    return { ...page, commands: [...page.commands], style_urls: [...page.style_urls] }
  })
}

export function pluginPageSession(raw: unknown): number | null {
  const text = typeof raw === 'string' ? raw : ''
  if (!/^[1-9]\d*$/.test(text)) return null
  const value = Number(text)
  return Number.isSafeInteger(value) ? value : null
}
