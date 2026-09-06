import { ApiError } from '../../lib/api'
import type { FsContentReplaceResponse, FsContentSearchFileResult } from './api/filesApi'

type ReplacementProgress = {
  completed: FsContentReplaceResponse
  failedPath: string
  remaining: number
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function count(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

export function contentReplacementProgress(error: unknown): ReplacementProgress | null {
  if (!(error instanceof ApiError)) return null
  const details = record(record(error.bodyJson)?.details)
  const completed = record(details?.completed)
  if (
    !details ||
    !completed ||
    typeof details.failedPath !== 'string' ||
    !count(details.remaining) ||
    typeof completed.root !== 'string' ||
    !count(completed.fileCount) ||
    !count(completed.replacementCount) ||
    !count(completed.skipped) ||
    typeof completed.truncated !== 'boolean' ||
    !Array.isArray(completed.files)
  )
    return null
  const files: FsContentReplaceResponse['files'] = []
  for (const raw of completed.files) {
    const file = record(raw)
    if (!file || typeof file.path !== 'string' || typeof file.relativePath !== 'string' || !count(file.replacements))
      return null
    files.push({ path: file.path, relativePath: file.relativePath, replacements: file.replacements })
  }
  if (
    files.length !== completed.fileCount ||
    files.reduce((sum, file) => sum + file.replacements, 0) !== completed.replacementCount
  )
    return null
  return {
    completed: {
      root: completed.root,
      fileCount: completed.fileCount,
      replacementCount: completed.replacementCount,
      skipped: completed.skipped,
      truncated: completed.truncated,
      files,
    },
    failedPath: details.failedPath,
    remaining: details.remaining,
  }
}

export function contentSearchRevisions(files: readonly FsContentSearchFileResult[]): Record<string, string> | null {
  if (files.some((file) => typeof file.revision !== 'string' || !/^[a-f0-9]{64}$/i.test(file.revision))) return null
  return Object.fromEntries(files.map((file) => [file.path, file.revision]))
}

type RefreshOptions = {
  directory: string
  // Undefined means the failure left the set of changed files uncertain.
  changedPaths?: readonly string[]
  current: () => { directory: string | null; path: string | null; dirty: boolean }
  normalizePath: (path: string) => string
  invalidate: (directory: string) => void
  refreshFile: () => Promise<unknown>
  refreshSearch: () => Promise<unknown>
}

export async function refreshContentReplacement(options: RefreshOptions): Promise<void> {
  options.invalidate(options.directory)
  const current = options.current()
  if (current.directory !== options.directory) return
  const selectedPath = current.path
  const changed =
    selectedPath &&
    (options.changedPaths === undefined ||
      options.changedPaths.some((path) => options.normalizePath(path) === options.normalizePath(selectedPath)))
  if (changed && !current.dirty) await options.refreshFile()
  if (options.current().directory === options.directory) await options.refreshSearch()
}
