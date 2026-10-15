import { fsPathStartsWith, normalizeFsPath, trimTrailingFsSlashes } from '@/lib/path'

export function queryText(value: unknown): string {
  if (Array.isArray(value)) return queryText(value.find((item) => queryText(item)))
  return typeof value === 'string' ? value.trim() : ''
}

export function resourceDirectory(query: Record<string, unknown>, fallback = ''): string {
  const explicit = queryText(query.directory)
  if (explicit) return trimTrailingFsSlashes(normalizeFsPath(explicit))
  const file = normalizeFsPath(queryText(query.filePath) || queryText(query.gitPath))
  const base = trimTrailingFsSlashes(normalizeFsPath(fallback))
  if (!file || !/^(\/|[A-Za-z]:\/)/.test(file) || (base && fsPathStartsWith(file, base))) return base
  // Legacy/deep file links without a workspace retain a usable absolute root.
  const slash = file.lastIndexOf('/')
  return trimTrailingFsSlashes(file.slice(0, slash + 1))
}
