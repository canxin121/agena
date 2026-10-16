export type SettingsSubpageDefinition = {
  id: string
  // Previous page ids keep bookmarks and remembered destinations working when
  // pages are renamed or merged. Resolution always returns the current id.
  aliases?: readonly string[]
  label: string
  description: string
  keywords?: string[]
  badge?: string
  // A dense master/detail page owns the whole settings content area instead of
  // scrolling inside it: SettingsPage and the section workbench give such a page
  // the viewport height and drop the centered content width cap.
  fillsViewport?: boolean
}

export function normalizeSettingsSubpageId(value: unknown): string {
  return String(value || '')
    .trim()
    .toLowerCase()
}

export function settingsSubpageStorageKey(section: string): string {
  const normalized = normalizeSettingsSubpageId(section) || 'unknown'
  return `studio.settings.subpage.${normalized}`
}

export function resolveSettingsSubpage(
  requested: unknown,
  remembered: unknown,
  pages: readonly SettingsSubpageDefinition[],
  fallback: string,
): string {
  const ids = new Map<string, string>()
  for (const page of pages) {
    const id = normalizeSettingsSubpageId(page.id)
    if (!id) continue
    ids.set(id, id)
    for (const alias of page.aliases || []) ids.set(normalizeSettingsSubpageId(alias), id)
  }
  const candidates = [requested, remembered, fallback, pages[0]?.id]
  for (const candidate of candidates) {
    const normalized = normalizeSettingsSubpageId(candidate)
    if (normalized && ids.has(normalized)) return ids.get(normalized)!
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
    const searchText = [page.id, ...(page.aliases || []), page.label, page.description, ...(page.keywords || [])]
      .join('\n')
      .toLowerCase()
    return searchText.includes(normalized)
  })
}
