export type SettingsSubpageDefinition = {
  id: string
  label: string
  description: string
  keywords?: string[]
  badge?: string
}

export function normalizeSettingsSubpageId(value: unknown): string {
  return String(value || '')
    .trim()
    .toLowerCase()
}

export function settingsSubpageStorageKey(section: string): string {
  const normalized = normalizeSettingsSubpageId(section) || 'unknown'
  return `studio.settings.subpage.${normalized}.v1`
}

export function resolveSettingsSubpage(
  requested: unknown,
  remembered: unknown,
  pages: readonly SettingsSubpageDefinition[],
  fallback: string,
): string {
  const ids = new Set(pages.map((page) => normalizeSettingsSubpageId(page.id)).filter(Boolean))
  const candidates = [requested, remembered, fallback, pages[0]?.id]
  for (const candidate of candidates) {
    const normalized = normalizeSettingsSubpageId(candidate)
    if (normalized && ids.has(normalized)) return normalized
  }
  return ''
}

export function filterSettingsSubpages(
  pages: readonly SettingsSubpageDefinition[],
  query: unknown,
): SettingsSubpageDefinition[] {
  const normalized = normalizeSettingsSubpageId(query)
  if (!normalized) return [...pages]
  return pages.filter((page) => {
    const searchText = [page.id, page.label, page.description, ...(page.keywords || [])].join('\n').toLowerCase()
    return searchText.includes(normalized)
  })
}
