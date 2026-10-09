export type PluginDocumentationTranslation = {
  summary?: string | null
  help?: string | null
}

export type ToolDocumentationTranslation = PluginDocumentationTranslation & {
  before_help?: string | null
  after_help?: string | null
}

export type LocalizableToolDocs = {
  before_help?: string | null
  after_help?: string | null
  summary?: string | null
  help?: string | null
  translations?: Record<string, ToolDocumentationTranslation | undefined>
}

export type LocalizableTool = {
  summary?: string | null
  description?: string | null
  docs?: LocalizableToolDocs | null
  [key: string]: unknown
}

export type LocalizablePluginManifest = {
  summary?: string | null
  help?: string | null
  translations?: Record<string, PluginDocumentationTranslation | undefined>
  tools?: LocalizableTool[]
  [key: string]: unknown
}

function localeCandidates(locale: string): string[] {
  const normalized = locale.trim().replaceAll('_', '-')
  const language = normalized.split('-')[0]
  return [...new Set([normalized, language, 'en-US', 'en'].filter(Boolean))]
}

export function localizedDocumentationField(
  base: string | null | undefined,
  translations: Record<string, { [field: string]: string | null | undefined } | undefined> | undefined,
  field: string,
  locale: string,
): string | undefined {
  for (const candidate of localeCandidates(locale)) {
    const entry = Object.entries(translations || {}).find(([tag]) => tag.toLowerCase() === candidate.toLowerCase())?.[1]
    const value = entry?.[field]
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return typeof base === 'string' && base.trim() ? base.trim() : undefined
}

export function localizePluginManifest<T extends LocalizablePluginManifest>(manifest: T, locale: string): T {
  const tools = Array.isArray(manifest.tools)
    ? manifest.tools.map((tool) => {
        const docs = tool.docs || {}
        const localizedDocs = {
          ...docs,
          before_help: localizedDocumentationField(docs.before_help, docs.translations, 'before_help', locale),
          after_help: localizedDocumentationField(docs.after_help, docs.translations, 'after_help', locale),
          summary: localizedDocumentationField(docs.summary, docs.translations, 'summary', locale),
          help: localizedDocumentationField(docs.help, docs.translations, 'help', locale),
        }
        return {
          ...tool,
          summary: localizedDocumentationField(tool.summary || docs.summary, docs.translations, 'summary', locale),
          docs: localizedDocs,
        }
      })
    : manifest.tools
  return {
    ...manifest,
    summary: localizedDocumentationField(manifest.summary, manifest.translations, 'summary', locale),
    help: localizedDocumentationField(manifest.help, manifest.translations, 'help', locale),
    tools,
  }
}
