import { apiJson } from '@/lib/api'

export type StudioHealth = {
  status: string
  generation: number
  loadedAt: string
  workspaceRoot: string
  configPath: string
  configFound: boolean
  activeMode?: string | null
  providerIds: string[]
  sessionRuntimeAvailable: boolean
}

export type RuntimeStatus = {
  generation: number
  loaded_at: string
  workspace_root: string
  config_path: string
  config_found: boolean
  active_mode?: string | null
  auth_store_path: string
  provider_ids: string[]
  plugin_count: number
  session_runtime_available: boolean
  watch_paths: string[]
  reload: {
    enabled: boolean
    interval_secs: number
  }
  janitor: {
    enabled: boolean
    interval_secs: number
  }
}

export type RuntimeReloadResponse = {
  cause: string
  previous_generation: number
  generation: number
  loaded_at: string
}

export type ProviderSummary = {
  provider_id: string
  default_model: string
  default_model_ref: string
}

export type AuthProvider = {
  provider_id: string
  configured: boolean
  credential_present: boolean
  credential_type?: string | null
  key_preview?: string | null
  expires_at?: string | null
  expired?: boolean | null
  account_id?: string | null
  enterprise_url?: string | null
}

export type WorkspaceResource = {
  id: number
  path: string
  created_at: string
  updated_at: string
  session_count?: number | null
}

export type SessionResource = {
  id: number
  parent_id?: number | null
  workspace_id: number
  title: string
  version: number
  created_at: string
  updated_at: string
  message_count: number
  child_session_count: number
  last_message_at?: string | null
}

export type MessagePart = {
  id: number
  message_id: number
  part_index: number
  status: string
  kind: string
  name?: string | null
  summary?: string | null
  has_detail?: boolean
  operation_id?: string | null
  created_at: string
  content?: Record<string, unknown> | null
}

export type MessageResource = {
  id: number
  session_id: number
  role: 'user' | 'assistant' | 'system' | 'tool'
  state: string
  created_at: string
  updated_at: string
  metadata: Record<string, unknown>
  usage?: Record<string, unknown> | null
  finish?: string | null
  part_count: number
  parts?: MessagePart[] | null
}

export type PermissionRequest = {
  request_id: string
  session_id?: number | null
  action: Record<string, unknown>
  reason: string
  created_at: string
}

export type UserInputQuestion = {
  id: string
  header?: string
  question: string
  options?: Array<{
    label: string
    description?: string
  }>
  multiple?: boolean
  allow_custom?: boolean
}

export type UserInputRequest = {
  request_id: string
  session_id?: number | null
  questions: UserInputQuestion[]
  created_at: string
}

export type SessionExecutionResource = {
  session: SessionResource
  blocked: boolean
  run_state: 'idle' | 'awaiting_model' | string
  latest_event_seq?: number | null
  pending_permission_requests: PermissionRequest[]
  pending_user_input_requests: UserInputRequest[]
}

type PaginatedResponse<T> = {
  items: T[]
  page?: {
    limit: number
    returned: number
    has_more: boolean
    next_cursor?: string | null
    order: 'asc' | 'desc'
  }
}

export async function fetchStudioHealth(): Promise<StudioHealth> {
  return await apiJson<StudioHealth>('/health')
}

export async function fetchRuntimeStatus(): Promise<RuntimeStatus> {
  return await apiJson<RuntimeStatus>('/api/v1/runtime')
}

export async function reloadRuntime(): Promise<RuntimeReloadResponse> {
  return await apiJson<RuntimeReloadResponse>('/api/v1/runtime/reload', {
    method: 'POST',
  })
}

export async function listProviders(): Promise<ProviderSummary[]> {
  return await apiJson<ProviderSummary[]>('/api/v1/providers')
}

export async function listAuthProviders(): Promise<AuthProvider[]> {
  return await apiJson<AuthProvider[]>('/api/v1/auth/providers')
}

export async function setProviderApiKey(providerId: string, apiKey: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/api-key`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ api_key: apiKey }),
  })
}

export async function deleteProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}`, {
    method: 'DELETE',
  })
}

export async function refreshProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/refresh`, {
    method: 'POST',
  })
}

export async function listWorkspaces(): Promise<WorkspaceResource[]> {
  const response = await apiJson<PaginatedResponse<WorkspaceResource>>('/api/v1/workspaces?limit=100')
  return response.items ?? []
}

export async function resolveWorkspace(path: string, createIfMissing: boolean): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces/resolve', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      path,
      create_if_missing: createIfMissing,
    }),
  })
}

export async function createWorkspace(path: string): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  })
}

export async function listSessions(workspaceId: number): Promise<SessionResource[]> {
  const response = await apiJson<PaginatedResponse<SessionResource>>(
    `/api/v1/sessions?workspace_id=${encodeURIComponent(String(workspaceId))}&limit=100`,
  )
  return response.items ?? []
}

export async function createSession(input: {
  workspaceId: number
  title: string
  parentId?: number | null
}): Promise<SessionResource> {
  return await apiJson<SessionResource>('/api/v1/sessions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      workspace_id: input.workspaceId,
      title: input.title,
      parent_id: input.parentId ?? null,
    }),
  })
}

export async function getSessionState(sessionId: number): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${sessionId}/state`)
}

export async function listMessages(sessionId: number): Promise<MessageResource[]> {
  const response = await apiJson<PaginatedResponse<MessageResource>>(
    `/api/v1/sessions/${sessionId}/messages?parts=full&limit=100`,
  )
  return response.items ?? []
}

export async function submitTurn(input: {
  sessionId: number
  text: string
  providerId?: string
  modelId?: string
}): Promise<SessionExecutionResource> {
  const body: Record<string, unknown> = {
    parts: [
      {
        type: 'text',
        text: input.text,
      },
    ],
  }

  if (input.providerId && input.modelId) {
    body.model = {
      provider_id: input.providerId,
      model_id: input.modelId,
    }
  }

  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/turns`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

export async function replyPermission(input: {
  sessionId: number
  requestId: string
  kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always'
  reason?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(
    `/api/v1/sessions/${input.sessionId}/permission-replies`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        reply: {
          request_id: input.requestId,
          kind: input.kind,
          ...(input.reason ? { reason: input.reason } : {}),
        },
      }),
    },
  )
}

export async function replyUserInput(input: {
  sessionId: number
  requestId: string
  answers: Record<string, string[]>
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(
    `/api/v1/sessions/${input.sessionId}/user-input-replies`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        reply: {
          request_id: input.requestId,
          kind: 'submit',
          answers: input.answers,
        },
      }),
    },
  )
}

export async function cancelUserInput(input: {
  sessionId: number
  requestId: string
  reason?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(
    `/api/v1/sessions/${input.sessionId}/user-input-replies`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        reply: {
          request_id: input.requestId,
          kind: 'cancel',
          ...(input.reason ? { reason: input.reason } : {}),
        },
      }),
    },
  )
}
