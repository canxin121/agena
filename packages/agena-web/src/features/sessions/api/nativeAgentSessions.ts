import { ref } from 'vue'

import { apiJson } from '@/lib/api'
import { sessionApiPath } from '@/lib/acpSession'
import { invalidateResources } from '@/lib/resourceSync'
import type { ExternalAgent, ExternalAgentCatalogEntry } from '@/types/chat'

const externalAgentCatalog = ref<ExternalAgentCatalogEntry[]>([
  { id: 'claude-code', name: 'Claude Code', builtin: true, command: 'claude-agent-acp' },
  { id: 'antigravity', name: 'Antigravity', builtin: true, command: 'agy', args: ['acp', '--process-per-session'] },
])

export function useExternalAgentCatalog() {
  return externalAgentCatalog
}

export async function listExternalAgents(signal?: AbortSignal): Promise<ExternalAgentCatalogEntry[]> {
  const entries = await apiJson<ExternalAgentCatalogEntry[]>('/api/v1/acp/agents', { signal })
  if (Array.isArray(entries)) {
    externalAgentCatalog.value = entries
  }
  return externalAgentCatalog.value
}

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
  title?: boolean
  delete?: boolean
  fork?: boolean
}

export type NativeIndexedSession = NativeAgentSession & {
  archived: boolean | null
  model: string | null
  state: string | null
  last_prompt_at: string | null
}

export type NativeHistoryPreview = {
  turns: { user: string; reply: string }[]
  next_before_turn: number | null
  omitted_content: boolean
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

export function renameNativeAgentSession(workspace: number, agent: ExternalAgent, nativeId: string, title: string) {
  return apiJson<{ kind: 'title'; native_id: string; title: string }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'set_title', agent, native_id: nativeId, title }),
    },
  )
}

export function deleteNativeAgentSession(workspace: number, agent: ExternalAgent, nativeId: string) {
  return apiJson<{ kind: 'deleted'; native_id: string }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'delete', agent, native_id: nativeId }),
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

export async function previewNativeAgentHistory(
  workspace: number,
  agent: ExternalAgent,
  nativeId: string,
  beforeTurn: number | null,
  signal?: AbortSignal,
) {
  const result = await apiJson<{ kind: 'history_preview'; preview: NativeHistoryPreview }>(
    `/api/v1/acp/workspaces/${workspace}/sessions`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ action: 'preview_history', agent, native_id: nativeId, before_turn: beforeTurn }),
      signal,
    },
  )
  return result.preview
}

export type ExternalAgentProviderTarget = {
  api_type: string
  base_url: string
}

export type ExternalAgentProvider = {
  provider_id: string
  supported: string[]
  required: boolean
  current: ExternalAgentProviderTarget | null
}

export type ExternalAgentProviders = {
  supported: boolean
  providers: ExternalAgentProvider[]
}

export type OpenAcpSession = {
  id: number
  workspace_id?: number
  title?: string
  external_agent?: ExternalAgent
  updated_at?: string
}

export async function listOpenAcpSessions(workspace: number, signal?: AbortSignal): Promise<OpenAcpSession[]> {
  const page = await apiJson<{ items?: OpenAcpSession[] }>(
    `/api/v1/acp/sessions?workspace_id=${encodeURIComponent(String(workspace))}&limit=50`,
    { signal },
  )
  return Array.isArray(page.items) ? page.items : []
}

export function getExternalAgentProviders(id: string, signal?: AbortSignal) {
  return apiJson<ExternalAgentProviders>(`${sessionApiPath(id)}/agent-providers`, { signal })
}

export async function manageNativeAgentSession(id: string, action: NativeSessionAction, signal?: AbortSignal) {
  const result = await apiJson<NativeSessionResult>(`${sessionApiPath(id)}/agent-session`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(action),
    signal,
  })
  if (action.action !== 'status') {
    const keys = ['session:' + id + ':state']
    if (action.action === 'rewind') keys.push('session:' + id + ':parts')
    invalidateResources(keys)
  }
  return result
}
