/**
 * Project catalog paging shared by every project-scoped surface.
 *
 * The sidebar owns the paged directory state; this module only keeps its page
 * contract in one place so a picker and the sidebar cannot disagree about page
 * size or which rows one page contains.
 */
import type { DirectoryEntry } from '@/features/sessions/model/types'

export const PROJECTS_PER_PAGE = 15

/** The page rows in catalog order, resolved against the directory map. */
export function directoriesForPage(
  rows: DirectoryEntry[],
  byId: Record<string, DirectoryEntry>,
): DirectoryEntry[] {
  return (Array.isArray(rows) ? rows : [])
    .map((entry) => byId[String(entry?.id || '')] || entry)
    .filter((entry): entry is DirectoryEntry => Boolean(entry && entry.path))
}
