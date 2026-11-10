/**
 * A paged, project-grouped view of the conversations this instance knows.
 *
 * Both pickers (the settings session scope and future session switchers) read
 * conversations from the same server API. Grouping by project and paging belong
 * here so a caller receives `OptionMenuGroup[]` rather than assembling ids.
 */
import { computed, ref } from 'vue'

import type { OptionMenuGroup } from '@/components/ui/optionMenu.types'
import type { Session } from '@/types/chat'
import { listSessions } from '@/stores/chat/api'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { settingsText as st } from '@/i18n/settingsText'

export type ProjectGroup = {
  workspaceId: number | null
  label: string
  sessions: Session[]
}

export const SESSIONS_PER_PAGE = 40

/** Project label of a session, falling back to its workspace id. */
export function sessionProjectLabel(session: Session | null | undefined, projectPath?: string | null): string {
  const path = String(projectPath || '').trim()
  if (path) return path
  const workspaceId = Number(session?.workspace_id)
  if (Number.isSafeInteger(workspaceId) && workspaceId > 0) return `#${workspaceId}`
  return st('Bot conversation')
}

export function groupSessionsByProject(
  sessions: Session[],
  projectPathFor: (session: Session) => string | null | undefined,
): ProjectGroup[] {
  const groups = new Map<string, ProjectGroup>()
  for (const session of sessions) {
    const id = String(session?.id || '').trim()
    if (!id) continue
    const workspaceId = Number.isSafeInteger(Number(session.workspace_id)) ? Number(session.workspace_id) : null
    const key = workspaceId === null ? 'global' : `workspace:${workspaceId}`
    const existing = groups.get(key)
    if (existing) {
      existing.sessions.push(session)
      continue
    }
    groups.set(key, {
      workspaceId,
      label: sessionProjectLabel(session, projectPathFor(session)),
      sessions: [session],
    })
  }
  // The server already returns conversations most recently updated first, so
  // insertion order keeps the busiest project at the top without a second sort.
  const ordered = [...groups.values()]
  const global = ordered.filter((group) => group.workspaceId === null)
  return [...ordered.filter((group) => group.workspaceId !== null), ...global]
}

/**
 * Feed grouped sessions into an `OptionMenuGroup[]`. Each project becomes one
 * collapsible group; the group header is not selectable, so selecting a session
 * always reports a real session id.
 */
export function sessionGroupsForPicker(
  groups: ProjectGroup[],
  opts: { selectedId?: string; labelForProject?: (project: ProjectGroup) => string } = {},
): OptionMenuGroup[] {
  return groups.map((project) => ({
    id: project.workspaceId === null ? 'global' : `workspace:${project.workspaceId}`,
    title: opts.labelForProject ? opts.labelForProject(project) : project.label,
    subtitle: st('{count} conversations', { count: project.sessions.length }),
    items: project.sessions.map((session) => ({
      id: String(session.id),
      label: String(session.title || st('Session {id}', { id: session.id })),
      description: `#${session.id}`,
      checked: String(session.id) === String(opts.selectedId || ''),
      keywords: `${session.id} ${session.title || ''}`,
    })),
  }))
}

/** Paged session list with the same grouping/paging contract as the sidebar. */
export function usePagedSessionGroups() {
  const sessions = ref<Session[]>([])
  const page = ref(1)
  const pageCount = ref(1)
  const loading = ref(false)
  const error = ref('')
  const cursorByPage = new Map<number, string | null>()
  let controller: AbortController | null = null

  async function load(targetPage = 1) {
    const next = Math.max(1, Math.floor(targetPage))
    controller?.abort()
    const request = new AbortController()
    controller = request
    loading.value = true
    error.value = ''
    try {
      const response = await listSessions({
        limit: SESSIONS_PER_PAGE,
        cursor: cursorByPage.get(next) || undefined,
        excludeSubagents: true,
        includeTotal: true,
        signal: request.signal,
      })
      if (controller !== request) return
      sessions.value = response.sessions
      page.value = next
      if (typeof response.total === 'number' && response.total > 0) {
        pageCount.value = Math.max(1, Math.ceil(response.total / SESSIONS_PER_PAGE))
      } else {
        pageCount.value = Math.max(1, next + (response.hasMore ? 1 : 0))
      }
      if (response.nextCursor) cursorByPage.set(next + 1, response.nextCursor)
    } catch (reason) {
      if (controller !== request || request.signal.aborted) return
      sessions.value = []
      error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      if (controller === request) loading.value = false
    }
  }

  const directories = useDirectorySessionStore()
  const pathFor = (session: Session): string | null => {
    const workspaceId = String(session.workspace_id ?? '').trim()
    if (!workspaceId) return null
    return directories.directoriesById[workspaceId]?.path || null
  }
  const groups = computed(() => groupSessionsByProject(sessions.value, pathFor))

  return { sessions, groups, page, pageCount, loading, error, load }
}
