import { apiJson } from '@/lib/api'
import { sessionApiPath } from '@/lib/acpSession'
import { invalidateResources } from '@/lib/resourceSync'
import type { ExternalAgent } from '@/types/chat'

export type NativeAgentSession = {
  native_id: string
  cwd: string
  title: string | null
  updated_at: string | null
}

type NativePage = {
  kind: 'index_page'
  sessions: NativeIndexedSession[]
  next_cursor: string | null
  capabilities: NativeIndexCapabilities
}

export type NativeArchiveFilter = 'unarchived' | 'archived' | 'all'

export type NativeIndexCapabilities = {
  index: boolean
  /** Public ACP archive support or the Claude Code AIR index extension. */
  archive: boolean
  batch_get: boolean
  watch: boolean
}

export type NativeIndexedSession = NativeAgentSession & {
  archived: boolean | null
  model: string | null
  state: string | null
  last_prompt_at: string | null
}

export type NativeSessionStatus = {
  kind: 'status'
  session: NativeAgentSession | null
  native_status: string | null
  additional_directories: string[]
  fork_points?: { message_id: number; label: string }[]
  rewind_points?: { message_id: number; label: string }[]
  capabilities: {
    status: boolean
    title: boolean
    close: boolean
    delete: boolean
    fork: boolean
    fork_before_message?: boolean
    rewind_before_message?: boolean
    continue_conversation?: boolean
    additional_directories: boolean
    system_prompt: boolean
  }
}

export type NativeSessionAction =
  | { action: 'status' }
  | { action: 'set_title'; title: string }
  | { action: 'set_directories'; directories: string[] }
  | { action: 'close' | 'delete' }
  | { action: 'fork'; title?: string; before_message_id?: number }
  | { action: 'rewind'; before_message_id: number }

type NativeSessionResult =
  | NativeSessionStatus
  | { kind: 'forked' | 'rewound'; session_id: number }
  | { kind: 'title'; title: string }
  | { kind: 'directories'; directories: string[] }
  | { kind: 'closed' | 'deleted' }

export function listNativeAgentSessions(
  workspace: number,
  agent: ExternalAgent,
  cursor: string | null,
  archived: NativeArchiveFilter,
  signal?: AbortSignal,
) {
  return apiJson<NativePage>(`/api/v1/acp/workspaces/${workspace}/sessions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ action: 'index_list', agent, cursor, archived }),
    signal,
  })
}

export function setNativeAgentArchived(workspace: number, agent: ExternalAgent, nativeId: string, archived: boolean) {
  return apiJson<{ kind: 'archive'; native_id: string; archived: boolean }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'set_archived', agent, native_id: nativeId, archived }),
    },
  )
}

export function watchNativeAgentSessions(workspace: number, agent: ExternalAgent, signal: AbortSignal) {
  return apiJson<{ kind: 'watch'; supported: boolean; changed: boolean }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'watch', agent }),
      signal,
    },
  )
}

export async function openNativeAgentSession(workspace: number, agent: ExternalAgent, nativeId: string, signal?: AbortSignal) {
  const result = await apiJson<{ kind: 'opened'; session_id: number }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'open', agent, native_id: nativeId }),
      signal,
    },
  )
  return result.session_id
}

export async function manageNativeAgentSession(id: string, action: NativeSessionAction, signal?: AbortSignal) {
  const result = await apiJson<NativeSessionResult>(`${sessionApiPath(id)}/agent-session`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(action),
    signal,
  })
  if (action.action !== 'status') {
    invalidateResources(['session:' + id + ':state'])
  }
  return result
}
