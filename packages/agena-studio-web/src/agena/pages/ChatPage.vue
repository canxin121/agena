<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'

import {
  cancelUserInput,
  createSession,
  createWorkspace,
  fetchRuntimeStatus,
  getSessionState,
  listMessages,
  listProviders,
  listSessions,
  listWorkspaces,
  replyPermission,
  replyUserInput,
  resolveWorkspace,
  streamSessionEvents,
  submitTurn,
  type SessionEventStreamHandle,
  type MessagePart,
  type MessageResource,
  type ProviderSummary,
  type RuntimeStatus,
  type SessionExecutionResource,
  type SessionResource,
  type WorkspaceResource,
} from '@/agena/lib/agenaApi'

const runtime = ref<RuntimeStatus | null>(null)
const providers = ref<ProviderSummary[]>([])
const workspaces = ref<WorkspaceResource[]>([])
const sessions = ref<SessionResource[]>([])
const messages = ref<MessageResource[]>([])
const sessionState = ref<SessionExecutionResource | null>(null)

const selectedWorkspaceId = ref<number | null>(null)
const selectedSessionId = ref<number | null>(null)
const workspacePath = ref('')
const newSessionTitle = ref('')
const composer = ref('')
const selectedProviderId = ref('')
const selectedModelId = ref('')
const loading = ref(false)
const sending = ref(false)
const errorMessage = ref('')

const userInputDrafts = reactive<Record<string, Record<string, string>>>({})

type RenderBlock = {
  body: string
  kind: 'text' | 'diff'
  summary?: string
}

let pollTimer: ReturnType<typeof setInterval> | null = null
let refreshTimer: ReturnType<typeof setTimeout> | null = null
let refreshInFlight = false
let refreshQueued = false
let eventStream: SessionEventStreamHandle | null = null

function providerDefaultModel(providerId: string): string {
  return providers.value.find((provider) => provider.provider_id === providerId)?.default_model || ''
}

function stopEventStream() {
  eventStream?.close()
  eventStream = null
}

function clearScheduledConversationRefresh() {
  refreshQueued = false
  if (!refreshTimer) return
  clearTimeout(refreshTimer)
  refreshTimer = null
}

function stopPolling() {
  if (!pollTimer) return
  clearInterval(pollTimer)
  pollTimer = null
}

function ensurePolling() {
  if (pollTimer || !selectedSessionId.value) return
  pollTimer = setInterval(() => {
    void refreshConversation(false)
  }, 1800)
}

function syncPolling() {
  if (eventStream) {
    stopPolling()
    return
  }

  if (!sessionState.value) {
    stopPolling()
    return
  }

  if (sessionState.value.blocked || sessionState.value.run_state !== 'idle') {
    ensurePolling()
    return
  }

  stopPolling()
}

function scheduleConversationRefresh(delayMs = 120) {
  if (!selectedSessionId.value || refreshTimer) return
  refreshTimer = setTimeout(() => {
    refreshTimer = null
    void refreshConversation(false)
  }, delayMs)
}

function syncEventStream() {
  const sessionId = selectedSessionId.value
  if (!sessionId) {
    stopEventStream()
    stopPolling()
    return
  }

  if (typeof ReadableStream === 'undefined' || typeof TextDecoder === 'undefined') {
    stopEventStream()
    syncPolling()
    return
  }

  if (eventStream) {
    return
  }

  eventStream = streamSessionEvents(sessionId, {
    afterSeq: sessionState.value?.latest_event_seq ?? 0,
    pollIntervalMs: 250,
    onOpen: () => {
      stopPolling()
    },
    onEvent: (event) => {
      if (selectedSessionId.value !== sessionId) return
      if (sessionState.value) {
        sessionState.value = {
          ...sessionState.value,
          latest_event_seq: Math.max(sessionState.value.latest_event_seq ?? 0, event.seq),
        }
      }
      if (applySessionEvent(event)) {
        scheduleConversationRefresh()
      }
    },
    onError: (error) => {
      if (selectedSessionId.value !== sessionId) return
      console.warn('session event stream failed', error)
    },
  })
}

function formatMessageTime(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString()
}

function partBody(part: MessagePart): string {
  const content = part.content || null
  if (!content) return part.summary || ''

  const type = typeof content.type === 'string' ? content.type : ''
  if (type === 'text' && typeof content.text === 'string') {
    return content.text
  }

  if (type === 'reasoning' && Array.isArray(content.summary)) {
    const summary = content.summary.filter((item): item is string => typeof item === 'string').join('\n')
    if (summary) return summary
  }

  if (type === 'command_execution') {
    const command = typeof content.command === 'string' ? content.command : ''
    const output = typeof content.output === 'string' ? content.output : ''
    return [command, output].filter((item) => item.trim().length > 0).join('\n\n') || part.summary || ''
  }

  if (type === 'error') {
    const code = typeof content.code === 'string' ? content.code : 'error'
    const message = typeof content.message === 'string' ? content.message : ''
    return `${code}: ${message}`.trim()
  }

  return part.summary || JSON.stringify(content, null, 2)
}

function partBlocks(part: MessagePart): RenderBlock[] {
  const content = part.content || null
  const applyPatch = applyPatchPayload(content)
  if (applyPatch) {
    const output = content ? readString(content.output_text) : null
    const diff = readString(applyPatch.diff)
    const blocks: RenderBlock[] = []
    if (output) {
      blocks.push({ body: output, kind: 'text' })
    }
    if (diff) {
      blocks.push({
        body: diff,
        kind: 'diff',
        summary: applyPatchDiffSummary(applyPatch),
      })
    }
    if (blocks.length) return blocks
  }

  const body = partBody(part)
  return body.trim().length > 0 ? [{ body, kind: 'text' }] : []
}

function messageBlocks(message: MessageResource): RenderBlock[] {
  const parts = Array.isArray(message.parts) ? message.parts : []
  if (!parts.length) return []
  return parts.flatMap((part) => partBlocks(part))
}

function applyPatchPayload(content: Record<string, unknown> | null): Record<string, unknown> | null {
  if (!content || content.type !== 'tool_execution') return null
  const details = asRecord(content.details)
  if (!details || details.source !== 'custom') return null
  const output = asRecord(details.output)
  if (!output || output.name !== 'apply_patch') return null
  return asRecord(output.payload)
}

function applyPatchDiffSummary(payload: Record<string, unknown>): string {
  const changes = Array.isArray(payload.changes) ? payload.changes : []
  if (!changes.length) return 'Patch diff'
  return `Patch diff (${changes.length} file${changes.length === 1 ? '' : 's'})`
}

function messageTags(message: MessageResource): string[] {
  const metadata = message.metadata as { tags?: unknown } | null
  const tags = metadata?.tags
  if (!Array.isArray(tags)) return []
  return tags.filter((tag): tag is string => typeof tag === 'string' && tag.trim().length > 0)
}

function messageUsageFacts(message: MessageResource): string[] {
  const usage = message.usage as Record<string, unknown> | null | undefined
  if (!usage) return []

  const facts: string[] = []
  const pushFact = (label: string, key: string) => {
    const value = usage[key]
    if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
      facts.push(`${label} ${value}`)
    }
  }

  pushFact('in', 'input_tokens')
  pushFact('out', 'output_tokens')
  pushFact('reasoning', 'reasoning_tokens')
  pushFact('cache read', 'cache_read_tokens')
  pushFact('cache write', 'cache_write_tokens')

  return facts
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function readFiniteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function sortMessages(items: MessageResource[]): MessageResource[] {
  return [...items].sort((left, right) => {
    const leftTime = Date.parse(left.created_at)
    const rightTime = Date.parse(right.created_at)
    if (Number.isFinite(leftTime) && Number.isFinite(rightTime) && leftTime !== rightTime) {
      return leftTime - rightTime
    }
    return left.id - right.id
  })
}

function sortMessageParts(items: MessagePart[]): MessagePart[] {
  return [...items].sort((left, right) => {
    if (left.part_index !== right.part_index) {
      return left.part_index - right.part_index
    }
    return left.id - right.id
  })
}

function applyMessagePartUpdatedEvent(payload: Record<string, unknown>): boolean {
  const sessionId = selectedSessionId.value
  const messageId = readFiniteNumber(payload.message_id)
  const messageRole = readString(payload.message_role) as MessageResource['role'] | null
  const messageState = readString(payload.message_state)
  const messageCreatedAt = readString(payload.message_created_at)
  const part = asRecord(payload.part) as MessagePart | null

  if (!sessionId || messageId === null || !messageRole || !messageState || !messageCreatedAt || !part) {
    return true
  }

  const nextMessages = messages.value.slice()
  const messageIndex = nextMessages.findIndex((message) => message.id === messageId)
  if (messageIndex < 0) {
    nextMessages.push({
      id: messageId,
      session_id: sessionId,
      role: messageRole,
      state: messageState,
      created_at: messageCreatedAt,
      updated_at: messageCreatedAt,
      metadata: {},
      usage: null,
      finish: null,
      part_count: 1,
      parts: [part],
    })
    messages.value = sortMessages(nextMessages)
    return part.status !== 'pending' || messageState !== 'pending'
  }

  const existing = nextMessages[messageIndex]
  const nextParts = Array.isArray(existing.parts) ? existing.parts.slice() : []
  const partIndex = nextParts.findIndex((item) => item.id === part.id)
  if (partIndex >= 0) {
    nextParts[partIndex] = part
  } else {
    nextParts.push(part)
  }

  nextMessages[messageIndex] = {
    ...existing,
    role: messageRole,
    state: messageState,
    created_at: messageCreatedAt,
    part_count: Math.max(existing.part_count, nextParts.length),
    parts: sortMessageParts(nextParts),
  }
  messages.value = sortMessages(nextMessages)
  return part.status !== 'pending' || messageState !== 'pending'
}

function applyMessagePartDeltaEvent(payload: Record<string, unknown>): boolean {
  const messageId = readFiniteNumber(payload.message_id)
  const partId = readFiniteNumber(payload.part_id)
  const field = readString(payload.field)
  const delta = typeof payload.delta === 'string' ? payload.delta : ''

  if (messageId === null || partId === null || !field) {
    return true
  }
  if (field !== 'text') {
    return true
  }

  const nextMessages = messages.value.slice()
  const messageIndex = nextMessages.findIndex((message) => message.id === messageId)
  if (messageIndex < 0) {
    return true
  }

  const existing = nextMessages[messageIndex]
  const nextParts = Array.isArray(existing.parts) ? existing.parts.slice() : []
  const targetIndex = nextParts.findIndex((part) => part.id === partId)
  if (targetIndex < 0) {
    return true
  }

  const target = nextParts[targetIndex]
  const content = asRecord(target.content)
  if (!content || content.type !== 'text') {
    return true
  }

  nextParts[targetIndex] = {
    ...target,
    content: {
      ...content,
      text: `${typeof content.text === 'string' ? content.text : ''}${delta}`,
    },
  }
  nextMessages[messageIndex] = {
    ...existing,
    parts: sortMessageParts(nextParts),
  }
  messages.value = sortMessages(nextMessages)
  return false
}

function applySessionEvent(event: SessionEventRecord): boolean {
  const payload = asRecord(event.payload)
  if (!payload) return true

  switch (event.event_type) {
    case 'message_part_updated':
      return applyMessagePartUpdatedEvent(payload)
    case 'message_part_delta':
      return applyMessagePartDeltaEvent(payload)
    default:
      return true
  }
}

function readUserAnswer(requestId: string, questionId: string): string {
  return userInputDrafts[requestId]?.[questionId] || ''
}

function updateUserAnswer(requestId: string, questionId: string, value: string) {
  ;(userInputDrafts[requestId] ||= {})[questionId] = value
}

async function loadSidebar() {
  const [runtimeData, providerData, workspaceData] = await Promise.all([
    fetchRuntimeStatus(),
    listProviders(),
    listWorkspaces(),
  ])

  runtime.value = runtimeData
  providers.value = providerData
  workspaces.value = workspaceData

  if (!selectedProviderId.value && providerData.length === 1) {
    selectedProviderId.value = providerData[0]?.provider_id || ''
    selectedModelId.value = providerData[0]?.default_model || ''
  }

  if (selectedWorkspaceId.value && workspaceData.some((workspace) => workspace.id === selectedWorkspaceId.value)) {
    await loadSessionsForWorkspace(selectedWorkspaceId.value, false)
    return
  }

  const firstWorkspace = workspaceData[0]
  if (firstWorkspace) {
    await selectWorkspace(firstWorkspace.id)
  }
}

async function loadSessionsForWorkspace(workspaceId: number, preserveSelection = true) {
  sessions.value = await listSessions(workspaceId)
  selectedWorkspaceId.value = workspaceId

  const currentSelectionStillExists =
    preserveSelection &&
    selectedSessionId.value !== null &&
    sessions.value.some((session) => session.id === selectedSessionId.value)

  if (currentSelectionStillExists && selectedSessionId.value !== null) {
    await refreshConversation(true)
    return
  }

  const firstSession = sessions.value[0]
  if (firstSession) {
    selectedSessionId.value = firstSession.id
    await refreshConversation(true)
    return
  }

  selectedSessionId.value = null
  messages.value = []
  sessionState.value = null
  stopEventStream()
  clearScheduledConversationRefresh()
  stopPolling()
}

async function selectWorkspace(workspaceId: number) {
  await loadSessionsForWorkspace(workspaceId, false)
}

async function selectSession(sessionId: number) {
  stopEventStream()
  clearScheduledConversationRefresh()
  selectedSessionId.value = sessionId
  await refreshConversation(true)
}

async function refreshConversation(foreground: boolean) {
  const sessionId = selectedSessionId.value
  if (!sessionId) return

  if (refreshInFlight) {
    refreshQueued = true
    return
  }

  if (foreground) {
    loading.value = true
  }
  refreshInFlight = true

  try {
    const [state, messageItems] = await Promise.all([getSessionState(sessionId), listMessages(sessionId)])
    if (selectedSessionId.value !== sessionId) return
    sessionState.value = state
    messages.value = messageItems
    syncEventStream()
    syncPolling()
  } catch (err) {
    if (selectedSessionId.value !== sessionId) return
    errorMessage.value = err instanceof Error ? err.message : String(err)
    stopPolling()
  } finally {
    refreshInFlight = false
    if (refreshQueued && selectedSessionId.value === sessionId) {
      refreshQueued = false
      scheduleConversationRefresh(0)
    }
    if (foreground) {
      loading.value = false
    }
  }
}

async function resolveWorkspaceAction(createIfMissing: boolean) {
  const path = workspacePath.value.trim()
  if (!path) return

  loading.value = true
  errorMessage.value = ''
  try {
    const workspace = createIfMissing ? await resolveWorkspace(path, true) : await createWorkspace(path)
    workspacePath.value = workspace.path
    await loadSidebar()
    await selectWorkspace(workspace.id)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function createSessionAction() {
  const workspaceId = selectedWorkspaceId.value
  if (!workspaceId) return

  loading.value = true
  errorMessage.value = ''
  try {
    const title = newSessionTitle.value.trim() || 'New session'
    const session = await createSession({
      workspaceId,
      title,
    })
    newSessionTitle.value = ''
    await loadSessionsForWorkspace(workspaceId, false)
    await selectSession(session.id)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function sendPrompt() {
  const sessionId = selectedSessionId.value
  const text = composer.value.trim()
  if (!sessionId || !text) return

  sending.value = true
  errorMessage.value = ''
  try {
    const state = await submitTurn({
      sessionId,
      text,
      providerId: selectedProviderId.value || undefined,
      modelId: selectedProviderId.value && selectedModelId.value ? selectedModelId.value : undefined,
    })
    sessionState.value = state
    composer.value = ''
    syncEventStream()
    await refreshConversation(false)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  } finally {
    sending.value = false
  }
}

async function approvePermission(requestId: string, kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always') {
  const sessionId = selectedSessionId.value
  if (!sessionId) return
  errorMessage.value = ''
  try {
    sessionState.value = await replyPermission({
      sessionId,
      requestId,
      kind,
    })
    syncEventStream()
    await refreshConversation(false)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  }
}

async function submitUserAnswers(requestId: string) {
  const sessionId = selectedSessionId.value
  if (!sessionId) return

  const request = sessionState.value?.pending_user_input_requests.find((item) => item.request_id === requestId)
  if (!request) return

  const answers: Record<string, string[]> = {}
  const draft = userInputDrafts[requestId] || {}
  for (const question of request.questions) {
    const raw = String(draft[question.id] || '').trim()
    if (!raw) continue
    answers[question.id] = question.multiple
      ? raw
          .split(',')
          .map((item) => item.trim())
          .filter(Boolean)
      : [raw]
  }

  try {
    sessionState.value = await replyUserInput({
      sessionId,
      requestId,
      answers,
    })
    syncEventStream()
    await refreshConversation(false)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  }
}

async function cancelUserAnswers(requestId: string) {
  const sessionId = selectedSessionId.value
  if (!sessionId) return

  try {
    sessionState.value = await cancelUserInput({
      sessionId,
      requestId,
      reason: 'Cancelled from Agena Studio',
    })
    syncEventStream()
    await refreshConversation(false)
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  }
}

const selectedWorkspace = computed(
  () => workspaces.value.find((workspace) => workspace.id === selectedWorkspaceId.value) || null,
)

const selectedSession = computed(() => sessions.value.find((session) => session.id === selectedSessionId.value) || null)

onMounted(() => {
  void loadSidebar().catch((err) => {
    errorMessage.value = err instanceof Error ? err.message : String(err)
  })
})

onBeforeUnmount(() => {
  stopEventStream()
  stopPolling()
  clearScheduledConversationRefresh()
})
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Chat</h1>
        <p class="page-description">
          Drive agena sessions directly through the native HTTP API. No legacy compatibility layer remains.
        </p>
      </div>
      <div class="badge">{{ runtime?.provider_ids?.length || 0 }} provider(s)</div>
    </header>

    <div v-if="errorMessage" class="notice">{{ errorMessage }}</div>

    <div class="split-layout">
      <aside class="stack">
        <section class="card">
          <h3>Workspace</h3>
          <div class="field">
            <label class="label" for="workspace-path">Path</label>
            <input id="workspace-path" v-model="workspacePath" class="input mono" placeholder="D:/git/ai/project" />
          </div>
          <div class="button-row" style="margin-top: 12px">
            <button
              class="button primary"
              :disabled="loading || !workspacePath.trim()"
              @click="resolveWorkspaceAction(true)"
            >
              Resolve or Create
            </button>
            <button class="button" :disabled="loading || !workspacePath.trim()" @click="resolveWorkspaceAction(false)">
              Create Only
            </button>
          </div>
        </section>

        <section class="card">
          <h3>Workspaces</h3>
          <div v-if="workspaces.length" class="list">
            <button
              v-for="workspace in workspaces"
              :key="workspace.id"
              class="list-item"
              :class="{ active: workspace.id === selectedWorkspaceId }"
              @click="selectWorkspace(workspace.id)"
            >
              <div>
                <strong>{{ workspace.path }}</strong>
              </div>
              <div class="muted">{{ workspace.session_count ?? 0 }} session(s)</div>
            </button>
          </div>
          <p v-else class="muted">No workspaces yet.</p>
        </section>

        <section class="card">
          <h3>Sessions</h3>
          <div class="field">
            <label class="label" for="session-title">Title</label>
            <input id="session-title" v-model="newSessionTitle" class="input" placeholder="New session" />
          </div>
          <div class="button-row" style="margin-top: 12px">
            <button class="button primary" :disabled="!selectedWorkspaceId || loading" @click="createSessionAction">
              Create Session
            </button>
          </div>
          <div v-if="sessions.length" class="list" style="margin-top: 14px">
            <button
              v-for="session in sessions"
              :key="session.id"
              class="list-item"
              :class="{ active: session.id === selectedSessionId }"
              @click="selectSession(session.id)"
            >
              <div>
                <strong>{{ session.title }}</strong>
              </div>
              <div class="muted">
                {{ session.message_count }} message(s) · updated {{ formatMessageTime(session.updated_at) }}
              </div>
            </button>
          </div>
          <p v-else class="muted" style="margin-top: 14px">No sessions in the selected workspace.</p>
        </section>
      </aside>

      <section class="stack">
        <section class="card">
          <h3>Active Session</h3>
          <div v-if="selectedSession">
            <div>
              <strong>{{ selectedSession.title }}</strong>
            </div>
            <div class="muted">workspace={{ selectedWorkspace?.path || 'unknown' }}</div>
            <div class="muted">
              run_state={{ sessionState?.run_state || 'unknown' }}, blocked={{
                sessionState?.blocked ? 'true' : 'false'
              }}
            </div>
          </div>
          <p v-else class="muted">Pick or create a session to start chatting.</p>
        </section>

        <section class="card">
          <h3>Run Options</h3>
          <div class="grid two">
            <div class="field">
              <label class="label" for="provider-select">Provider</label>
              <select
                id="provider-select"
                v-model="selectedProviderId"
                class="select"
                @change="selectedModelId = providerDefaultModel(selectedProviderId)"
              >
                <option value="">Auto</option>
                <option v-for="provider in providers" :key="provider.provider_id" :value="provider.provider_id">
                  {{ provider.provider_id }}
                </option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="model-id">Model</label>
              <input id="model-id" v-model="selectedModelId" class="input mono" placeholder="gpt-5" />
            </div>
          </div>
        </section>

        <section class="card">
          <div class="page-header" style="margin-bottom: 12px">
            <h3 style="margin: 0">Messages</h3>
            <div class="button-row">
              <button class="button ghost" :disabled="!selectedSessionId || loading" @click="refreshConversation(true)">
                Refresh
              </button>
            </div>
          </div>

          <div v-if="messages.length" class="message-list">
            <article v-for="message in messages" :key="message.id" class="message" :class="message.role">
              <div class="message-head">
                <div class="message-role">{{ message.role }}</div>
                <div>{{ formatMessageTime(message.created_at) }}</div>
              </div>
              <div
                v-if="messageTags(message).length || messageUsageFacts(message).length || message.finish"
                class="stack"
              >
                <div v-if="messageTags(message).length" class="button-row">
                  <span v-for="tag in messageTags(message)" :key="`${message.id}-tag-${tag}`" class="badge">
                    {{ tag }}
                  </span>
                </div>
                <div v-if="messageUsageFacts(message).length" class="muted mono">
                  usage={{ messageUsageFacts(message).join(' · ') }}
                </div>
                <div v-if="message.finish" class="muted mono">finish={{ message.finish }}</div>
              </div>
              <div v-if="messageBlocks(message).length" class="stack">
                <template v-for="(block, index) in messageBlocks(message)" :key="`${message.id}-${index}`">
                  <details v-if="block.kind === 'diff'" class="message-diff">
                    <summary>{{ block.summary || 'Patch diff' }}</summary>
                    <pre class="message-block mono">{{ block.body }}</pre>
                  </details>
                  <pre v-else class="message-block mono">{{ block.body }}</pre>
                </template>
              </div>
              <div v-else class="muted">No renderable parts.</div>
            </article>
          </div>
          <p v-else class="muted">No messages yet.</p>
        </section>

        <section v-if="sessionState?.pending_permission_requests?.length" class="card">
          <h3>Pending Permissions</h3>
          <div class="list">
            <div
              v-for="request in sessionState.pending_permission_requests"
              :key="request.request_id"
              class="list-item"
            >
              <div>
                <strong>{{ request.request_id }}</strong>
              </div>
              <div class="muted">{{ request.reason }}</div>
              <pre class="message-block mono">{{ JSON.stringify(request.action, null, 2) }}</pre>
              <div class="button-row">
                <button class="button primary" @click="approvePermission(request.request_id, 'allow_once')">
                  Allow Once
                </button>
                <button class="button" @click="approvePermission(request.request_id, 'allow_always')">
                  Allow Always
                </button>
                <button class="button danger" @click="approvePermission(request.request_id, 'deny_once')">Deny</button>
              </div>
            </div>
          </div>
        </section>

        <section v-if="sessionState?.pending_user_input_requests?.length" class="card">
          <h3>Pending User Input</h3>
          <div class="list">
            <div
              v-for="request in sessionState.pending_user_input_requests"
              :key="request.request_id"
              class="list-item"
            >
              <div>
                <strong>{{ request.request_id }}</strong>
              </div>
              <div class="stack" style="margin-top: 10px">
                <div v-for="question in request.questions" :key="question.id" class="field">
                  <label class="label" :for="`${request.request_id}-${question.id}`">
                    {{ question.header || question.question }}
                  </label>
                  <textarea
                    :id="`${request.request_id}-${question.id}`"
                    class="textarea"
                    :value="readUserAnswer(request.request_id, question.id)"
                    :placeholder="question.multiple ? 'comma,separated,values' : question.question"
                    @input="
                      updateUserAnswer(
                        request.request_id,
                        question.id,
                        ($event.target as HTMLTextAreaElement | null)?.value || '',
                      )
                    "
                  />
                </div>
              </div>
              <div class="button-row" style="margin-top: 12px">
                <button class="button primary" @click="submitUserAnswers(request.request_id)">Submit Answers</button>
                <button class="button danger" @click="cancelUserAnswers(request.request_id)">Cancel</button>
              </div>
            </div>
          </div>
        </section>

        <section class="card">
          <h3>Composer</h3>
          <div class="field">
            <label class="label" for="composer">Prompt</label>
            <textarea
              id="composer"
              v-model="composer"
              class="textarea mono"
              placeholder="Ask agena to inspect the repo, plan a change, or run tools."
            />
          </div>
          <div class="button-row" style="margin-top: 12px">
            <button
              class="button primary"
              :disabled="sending || !selectedSessionId || !composer.trim()"
              @click="sendPrompt"
            >
              {{ sending ? 'Sending…' : 'Send Prompt' }}
            </button>
          </div>
        </section>
      </section>
    </div>
  </section>
</template>
