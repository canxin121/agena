import type {
  NativeIndexCapabilities,
  NativeIndexedSession,
} from '@/features/sessions/api/nativeAgentSessions'
import { directoryEntryLabel } from '@/features/sessions/model/labels'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { ExternalAgent } from '@/types/chat'

export type AcpDraftSession = {
  sessionId: string
  workspaceId: number
  agent: ExternalAgent
  title: string
  updatedAtMs: number
}

export type AcpConversationRow = {
  key: string
  kind: 'native' | 'draft'
  workspaceId: number
  agent: ExternalAgent
  nativeId: string | null
  sessionId: string | null
  title: string
  updatedAtMs: number
  state: string | null
  archived: boolean
  capabilities: NativeIndexCapabilities | null
}

export type AcpDirectoryActivityState = 'running' | 'attention' | 'mixed' | null

export function parseTimestampMs(value: string | null | undefined): number {
  if (!value) return 0
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

export function mergeProjectConversations(params: {
  workspaceId: number
  agent: ExternalAgent
  nativeSessions: readonly NativeIndexedSession[]
  capabilities: NativeIndexCapabilities | null
  drafts: readonly AcpDraftSession[]
  sessionIdByNativeId: ReadonlyMap<string, string>
  nativeIdBySessionId: ReadonlyMap<string, string>
  query?: string
}): AcpConversationRow[] {
  const {
    workspaceId,
    agent,
    nativeSessions,
    capabilities,
    drafts,
    sessionIdByNativeId,
    nativeIdBySessionId,
    query = '',
  } = params

  const nativeIds = new Set<string>()
  const linkedSessionIds = new Set<string>()
  const nativeRows: AcpConversationRow[] = nativeSessions.map((session) => {
    nativeIds.add(session.native_id)
    const mappedSessionId = sessionIdByNativeId.get(session.native_id) ?? null
    if (mappedSessionId) linkedSessionIds.add(mappedSessionId)
    return {
      key: `native:${session.native_id}`,
      kind: 'native',
      workspaceId,
      agent,
      nativeId: session.native_id,
      sessionId: mappedSessionId,
      title: session.title?.trim() || session.native_id,
      updatedAtMs: parseTimestampMs(session.updated_at || session.last_prompt_at),
      state: session.state,
      archived: session.archived === true,
      capabilities,
    }
  })

  const draftRows: AcpConversationRow[] = []
  for (const draft of drafts) {
    if (draft.workspaceId !== workspaceId || draft.agent !== agent) continue
    const linkedNative = nativeIdBySessionId.get(draft.sessionId)
    if (linkedNative && nativeIds.has(linkedNative)) continue
    if (linkedSessionIds.has(draft.sessionId)) continue
    draftRows.push({
      key: `draft:${draft.sessionId}`,
      kind: 'draft',
      workspaceId,
      agent,
      nativeId: linkedNative ?? null,
      sessionId: draft.sessionId,
      title: draft.title.trim() || `#${draft.sessionId}`,
      updatedAtMs: draft.updatedAtMs,
      state: null,
      archived: false,
      capabilities: null,
    })
  }

  draftRows.sort((a, b) => b.updatedAtMs - a.updatedAtMs)
  const merged = [...draftRows, ...nativeRows]

  const norm = query.trim().toLowerCase()
  if (!norm) return merged
  return merged.filter((row) =>
    [row.title, row.nativeId || '', row.sessionId || ''].some((value) => value.toLowerCase().includes(norm)),
  )
}

export function filterAcpDirectories(
  directories: readonly DirectoryEntry[],
  query: string,
  hasMatchingRows: (directoryId: string) => boolean,
): DirectoryEntry[] {
  const norm = query.trim().toLowerCase()
  if (!norm) return [...directories]
  return directories.filter((dir) => {
    const label = directoryEntryLabel(dir).toLowerCase()
    const path = (dir.path || '').toLowerCase()
    return label.includes(norm) || path.includes(norm) || hasMatchingRows(dir.id)
  })
}

export function acpSessionStatusDot(state: string | null): { dotClass: string; labelKey: string } {
  if (state === 'running') {
    return { dotClass: 'bg-primary animate-pulse', labelKey: 'chat.externalAgentDetails.states.running' }
  }
  if (state === 'requires_action') {
    return { dotClass: 'bg-amber-500', labelKey: 'chat.externalAgentDetails.states.requires_action' }
  }
  return { dotClass: '', labelKey: 'chat.sidebar.sessionRow.status.idle' }
}

export function acpDirectoryActivityState(rows: readonly AcpConversationRow[]): AcpDirectoryActivityState {
  let hasRunning = false
  let hasAttention = false
  for (const row of rows) {
    if (row.state === 'running') hasRunning = true
    else if (row.state === 'requires_action') hasAttention = true
    if (hasRunning && hasAttention) return 'mixed'
  }
  if (hasAttention) return 'attention'
  if (hasRunning) return 'running'
  return null
}
