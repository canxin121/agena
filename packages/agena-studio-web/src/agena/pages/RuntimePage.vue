<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import {
  createPermissionRule,
  replyPermission,
  revokePermissionRule,
  deleteProviderCredential,
  fetchRuntimeStatus,
  getPlugin,
  getSessionState,
  listAuthProviders,
  listPermissionRules,
  listPluginLogs,
  listPlugins,
  listProviderModels,
  listProviders,
  listSessionTimeline,
  listSessions,
  listWorkspaces,
  refreshProviderCredential,
  reloadRuntime,
  setProviderApiKey,
  updatePermissionRule,
  type AuthProvider,
  type PermissionMode,
  type PermissionRuleResource,
  type PluginInspect,
  type PluginLogEntry,
  type PluginStatus,
  type ProviderModel,
  type ProviderSummary,
  type RuntimeSkill,
  type RuntimeStatus,
  type SessionExecutionResource,
  type SessionResource,
  type TimelineEventRecord,
  type WorkspaceResource,
} from '@/agena/lib/agenaApi'
import {
  permissionActionView,
  permissionExplainability,
  permissionReplyPreview,
  permissionRiskLabel,
} from '@/agena/lib/permissionFormatting'
import {
  buildAuthProviderFacts,
  buildExecutionFacts,
  buildOperatorCards,
  buildRuntimeSnapshotFacts,
  buildSessionCacheFacts,
  buildTimelineSummary,
  formatProviderModel,
  mergePluginLogs,
  pickNextPluginId,
  pluginLogCursor,
} from './runtimePageModel'

type RuntimeTab = 'overview' | 'workflow' | 'plugins' | 'mcp' | 'lsp' | 'skills' | 'operator'

const tabs: Array<{ id: RuntimeTab; label: string }> = [
  { id: 'overview', label: 'Overview' },
  { id: 'workflow', label: 'Workflow' },
  { id: 'plugins', label: 'Plugins' },
  { id: 'mcp', label: 'MCP' },
  { id: 'lsp', label: 'LSP' },
  { id: 'skills', label: 'Skills' },
  { id: 'operator', label: 'Operator' },
]

const router = useRouter()
const activeTab = ref<RuntimeTab>('overview')
const runtime = ref<RuntimeStatus | null>(null)
const providers = ref<ProviderSummary[]>([])
const providerModels = reactive<Record<string, ProviderModel[]>>({})
const authProviders = ref<AuthProvider[]>([])
const permissionRules = ref<PermissionRuleResource[]>([])
const plugins = ref<PluginStatus[]>([])
const workspaces = ref<WorkspaceResource[]>([])
const sessions = ref<SessionResource[]>([])
const selectedWorkspaceId = ref<number | null>(null)
const selectedSessionId = ref<number | null>(null)
const sessionExecution = ref<SessionExecutionResource | null>(null)
const sessionTimeline = ref<TimelineEventRecord[]>([])
const selectedPluginId = ref('')
const selectedPlugin = ref<PluginInspect | null>(null)
const pluginLogs = ref<PluginLogEntry[]>([])
const pluginLogPollTimer = ref<ReturnType<typeof setInterval> | null>(null)
const loading = ref(false)
const pluginLoading = ref(false)
const workflowLoading = ref(false)
const actionError = ref('')
const actionMessage = ref('')
const drafts = reactive<Record<string, string>>({})
const permissionSearch = ref('')
const permissionModeFilter = ref<'all' | PermissionMode>('all')
const permissionScopeFilter = ref<'all' | 'session' | 'workspace' | 'global'>('all')
const permissionSubjectFilter = ref<'all' | 'builtin_tool' | 'path_access'>('all')
const permissionStatusFilter = ref<'all' | 'active' | 'revoked'>('active')
const permissionDraft = reactive<{
  subjectKind: 'builtin_tool' | 'path_access'
  toolName: string
  qualifier: string
  pathAccessKind: string
  workspaceRoot: string
  targetPath: string
  scope: 'session' | 'workspace' | 'global'
  sessionId: string
  mode: PermissionMode
}>({
  subjectKind: 'builtin_tool',
  toolName: '',
  qualifier: '',
  pathAccessKind: 'read',
  workspaceRoot: '',
  targetPath: '',
  scope: 'workspace',
  sessionId: '',
  mode: 'ask',
})
const editingPermissionRuleId = ref<number | null>(null)

const operatorCards = computed(() => buildOperatorCards(runtime.value))
const runtimeSnapshotFacts = computed(() => buildRuntimeSnapshotFacts(runtime.value))
const sessionCacheFacts = computed(() => buildSessionCacheFacts(runtime.value))
const executionFacts = computed(() => buildExecutionFacts(sessionExecution.value))
const timelineSummaries = computed(() => buildTimelineSummary(sessionTimeline.value))

const skillCommands = computed<RuntimeSkill[]>(() => runtime.value?.operator.skills.commands ?? [])
const discoveredSkills = computed<RuntimeSkill[]>(() => runtime.value?.operator.skills.skills ?? [])
const filteredPermissionRules = computed(() => {
  return permissionRules.value.filter((rule) => {
    if (permissionModeFilter.value !== 'all' && rule.mode !== permissionModeFilter.value) return false
    if (permissionScopeFilter.value !== 'all' && rule.scope !== permissionScopeFilter.value) return false
    if (permissionSubjectFilter.value !== 'all' && rule.subject_kind !== permissionSubjectFilter.value) return false
    if (permissionStatusFilter.value === 'active' && rule.revoked_at) return false
    if (permissionStatusFilter.value === 'revoked' && !rule.revoked_at) return false
    return true
  })
})

async function load() {
  loading.value = true
  actionError.value = ''
  try {
    const [runtimeData, providerData, authData, permissionRuleData, pluginData, workspaceData] = await Promise.all([
      fetchRuntimeStatus(),
      listProviders(),
      listAuthProviders(),
      listPermissionRules(permissionSearch.value),
      listPlugins(),
      listWorkspaces(),
    ])
    runtime.value = runtimeData
    providers.value = providerData
    authProviders.value = authData
    permissionRules.value = permissionRuleData
    plugins.value = pluginData
    workspaces.value = workspaceData

    await Promise.all(
      providerData.map(async (provider) => {
        providerModels[provider.provider_id] = await listProviderModels(provider.provider_id)
      }),
    )

    const nextWorkspaceId = pickWorkspaceId(selectedWorkspaceId.value, workspaceData)
    selectedWorkspaceId.value = nextWorkspaceId
    sessions.value = nextWorkspaceId ? await listSessions(nextWorkspaceId) : []
    const nextSessionId = pickSessionId(selectedSessionId.value, sessions.value)
    selectedSessionId.value = nextSessionId
    if (nextSessionId) {
      await loadSessionExecution(nextSessionId)
    } else {
      sessionExecution.value = null
      sessionTimeline.value = []
    }

    const nextPluginId = pickNextPluginId(selectedPluginId.value, pluginData)
    selectedPluginId.value = nextPluginId
    if (nextPluginId) {
      await loadPluginDetails(nextPluginId)
    } else {
      selectedPlugin.value = null
      pluginLogs.value = []
      stopPluginLogPolling()
    }
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function loadPluginDetails(pluginId: string) {
  if (!pluginId) {
    selectedPlugin.value = null
    pluginLogs.value = []
    stopPluginLogPolling()
    return
  }
  pluginLoading.value = true
  actionError.value = ''
  try {
    const [plugin, logs] = await Promise.all([getPlugin(pluginId), listPluginLogs(pluginId, { limit: 50 })])
    selectedPluginId.value = pluginId
    selectedPlugin.value = plugin
    pluginLogs.value = logs
    syncPluginLogPolling()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    pluginLoading.value = false
  }
}

async function triggerReload() {
  actionMessage.value = ''
  actionError.value = ''
  try {
    const result = await reloadRuntime()
    actionMessage.value = `Runtime reloaded to generation ${result.generation}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function loadSessionExecution(sessionId: number) {
  workflowLoading.value = true
  actionError.value = ''
  try {
    const [execution, timeline] = await Promise.all([
      getSessionState(sessionId),
      listSessionTimeline(sessionId, { limit: 25 }),
    ])
    if (selectedSessionId.value !== sessionId) return
    sessionExecution.value = execution
    sessionTimeline.value = timeline
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    workflowLoading.value = false
  }
}

async function selectWorkspace(workspaceId: number) {
  selectedWorkspaceId.value = workspaceId
  sessions.value = await listSessions(workspaceId)
  const nextSessionId = pickSessionId(selectedSessionId.value, sessions.value)
  selectedSessionId.value = nextSessionId
  if (nextSessionId) {
    await loadSessionExecution(nextSessionId)
  } else {
    sessionExecution.value = null
    sessionTimeline.value = []
  }
}

async function selectSession(sessionId: number) {
  selectedSessionId.value = sessionId
  await loadSessionExecution(sessionId)
}

function openSelectedSessionInChat() {
  if (!selectedSessionId.value) return
  void router.push(`/chat?session=${selectedSessionId.value}`)
}

function pickWorkspaceId(currentWorkspaceId: number | null, items: WorkspaceResource[]): number | null {
  if (currentWorkspaceId && items.some((workspace) => workspace.id === currentWorkspaceId)) {
    return currentWorkspaceId
  }
  return items[0]?.id ?? null
}

function pickSessionId(currentSessionId: number | null, items: SessionResource[]): number | null {
  if (currentSessionId && items.some((session) => session.id === currentSessionId)) {
    return currentSessionId
  }
  return items[0]?.id ?? null
}

function stopPluginLogPolling() {
  if (!pluginLogPollTimer.value) return
  clearInterval(pluginLogPollTimer.value)
  pluginLogPollTimer.value = null
}

function syncPluginLogPolling() {
  stopPluginLogPolling()
  if (activeTab.value !== 'plugins' || !selectedPluginId.value) return
  pluginLogPollTimer.value = setInterval(() => {
    void refreshPluginLogsIncrementally()
  }, 1_500)
}

async function refreshPluginLogsIncrementally() {
  const pluginId = selectedPluginId.value
  if (!pluginId) return
  const afterSeq = pluginLogCursor(pluginLogs.value)
  const incoming = await listPluginLogs(pluginId, {
    limit: 50,
    ...(afterSeq != null ? { afterSeq } : {}),
  })
  if (!incoming.length) return
  pluginLogs.value = mergePluginLogs(pluginLogs.value, incoming)
}

async function saveApiKey(providerId: string) {
  const apiKey = String(drafts[providerId] || '').trim()
  if (!apiKey) return
  actionMessage.value = ''
  actionError.value = ''
  try {
    await setProviderApiKey(providerId, apiKey)
    drafts[providerId] = ''
    actionMessage.value = `Saved API key for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function clearCredential(providerId: string) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await deleteProviderCredential(providerId)
    actionMessage.value = `Cleared credential for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function refreshCredential(providerId: string) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await refreshProviderCredential(providerId)
    actionMessage.value = `Requested credential refresh for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

function resetPermissionDraft() {
  permissionDraft.subjectKind = 'builtin_tool'
  permissionDraft.toolName = ''
  permissionDraft.qualifier = ''
  permissionDraft.pathAccessKind = 'read'
  permissionDraft.workspaceRoot = ''
  permissionDraft.targetPath = ''
  permissionDraft.scope = 'workspace'
  permissionDraft.sessionId = ''
  permissionDraft.mode = 'ask'
  editingPermissionRuleId.value = null
}

function permissionRuleLabel(rule: PermissionRuleResource): string {
  if (rule.subject_kind === 'builtin_tool') {
    return rule.qualifier?.trim() ? `${rule.tool_name} · ${rule.qualifier}` : rule.tool_name || rule.action_key
  }
  if (rule.subject_kind === 'path_access') {
    return `${rule.path_access_kind || 'path'} · ${rule.target_path || rule.action_key}`
  }
  return rule.action_key
}

function permissionRuleScopeLabel(rule: PermissionRuleResource): string {
  if (rule.scope === 'session') {
    return rule.session_id == null ? 'session' : `session #${rule.session_id}`
  }
  if (rule.scope === 'workspace') {
    return rule.workspace_id == null ? 'workspace' : `workspace #${rule.workspace_id}`
  }
  if (rule.scope === 'global') {
    return 'global'
  }
  return rule.scope
}

function permissionRuleFacts(rule: PermissionRuleResource): string[] {
  const facts = [
    `scope=${permissionRuleScopeLabel(rule)}`,
    `source=${rule.source}`,
    `status=${rule.revoked_at ? 'revoked' : 'active'}`,
  ]
  if (rule.operator) facts.push(`operator=${rule.operator}`)
  if (rule.reason) facts.push(`reason=${rule.reason}`)
  if (rule.revoked_at) facts.push(`revoked_at=${rule.revoked_at}`)
  if (rule.revoked_reason) facts.push(`revoked_reason=${rule.revoked_reason}`)
  if (rule.revoked_by) facts.push(`revoked_by=${rule.revoked_by}`)
  return facts
}

function permissionRulePreview(rule: PermissionRuleResource): string {
  if (rule.subject_kind === 'builtin_tool') {
    const qualifier = rule.qualifier?.trim()
    return qualifier ? `tool=${rule.tool_name} · qualifier=${qualifier}` : `tool=${rule.tool_name}`
  }
  return [
    `access=${rule.path_access_kind || 'path_access'}`,
    rule.workspace_root ? `workspace=${rule.workspace_root}` : null,
    rule.target_path ? `target=${rule.target_path}` : null,
  ]
    .filter(Boolean)
    .join(' · ')
}

function editPermissionRule(rule: PermissionRuleResource) {
  permissionDraft.subjectKind = rule.subject_kind === 'path_access' ? 'path_access' : 'builtin_tool'
  permissionDraft.toolName = rule.tool_name || ''
  permissionDraft.qualifier = rule.qualifier || ''
  permissionDraft.pathAccessKind = rule.path_access_kind || 'read'
  permissionDraft.workspaceRoot = rule.workspace_root || ''
  permissionDraft.targetPath = rule.target_path || ''
  permissionDraft.scope = rule.scope === 'session' ? 'session' : rule.scope === 'global' ? 'global' : 'workspace'
  permissionDraft.sessionId = rule.session_id == null ? '' : String(rule.session_id)
  permissionDraft.mode = rule.mode
  editingPermissionRuleId.value = rule.id
}

async function savePermissionRule() {
  const toolName = permissionDraft.toolName.trim()
  const qualifier = permissionDraft.qualifier.trim()
  const targetPath = permissionDraft.targetPath.trim()
  if (permissionDraft.subjectKind === 'builtin_tool' && !toolName) return
  if (permissionDraft.subjectKind === 'path_access' && !targetPath) return

  const payload = {
    subjectKind: permissionDraft.subjectKind,
    toolName: permissionDraft.subjectKind === 'builtin_tool' ? toolName : undefined,
    qualifier: permissionDraft.subjectKind === 'builtin_tool' && qualifier ? qualifier : undefined,
    pathAccessKind: permissionDraft.subjectKind === 'path_access' ? permissionDraft.pathAccessKind : undefined,
    workspaceRoot:
      permissionDraft.subjectKind === 'path_access' && permissionDraft.workspaceRoot.trim()
        ? permissionDraft.workspaceRoot.trim()
        : undefined,
    targetPath: permissionDraft.subjectKind === 'path_access' ? targetPath : undefined,
    scope: permissionDraft.scope,
    sessionId:
      permissionDraft.scope === 'session' && permissionDraft.sessionId.trim()
        ? Number(permissionDraft.sessionId.trim())
        : undefined,
    mode: permissionDraft.mode,
  } as const

  actionMessage.value = ''
  actionError.value = ''
  try {
    if (editingPermissionRuleId.value) {
      await updatePermissionRule({
        id: editingPermissionRuleId.value,
        ...payload,
      })
      actionMessage.value = `Updated permission rule for ${permissionRuleLabel({
        id: editingPermissionRuleId.value,
        action_key: '',
        subject_kind: payload.subjectKind,
        tool_name: payload.toolName ?? null,
        qualifier: payload.qualifier ?? null,
        path_access_kind: payload.pathAccessKind ?? null,
        workspace_root: payload.workspaceRoot ?? null,
        target_path: payload.targetPath ?? null,
        mode: payload.mode,
        scope: payload.scope,
        source: 'api',
        created_at: '',
        updated_at: '',
      } as PermissionRuleResource)}.`
    } else {
      await createPermissionRule(payload)
      actionMessage.value = `Created permission rule for ${permissionRuleLabel({
        id: 0,
        action_key: '',
        subject_kind: payload.subjectKind,
        tool_name: payload.toolName ?? null,
        qualifier: payload.qualifier ?? null,
        path_access_kind: payload.pathAccessKind ?? null,
        workspace_root: payload.workspaceRoot ?? null,
        target_path: payload.targetPath ?? null,
        mode: payload.mode,
        scope: payload.scope,
        source: 'api',
        created_at: '',
        updated_at: '',
      } as PermissionRuleResource)}.`
    }
    resetPermissionDraft()
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function revokePermissionRuleAction(rule: PermissionRuleResource) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await revokePermissionRule(rule.id)
    actionMessage.value = `Revoked permission rule for ${permissionRuleLabel(rule)}.`
    if (editingPermissionRuleId.value === rule.id) resetPermissionDraft()
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function approvePermission(
  requestId: string,
  kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
  scope?: 'session' | 'workspace' | 'global',
) {
  const sessionId = selectedSessionId.value
  if (!sessionId) return
  actionMessage.value = ''
  actionError.value = ''
  try {
    sessionExecution.value = await replyPermission({
      sessionId,
      requestId,
      kind,
      scope,
    })
    actionMessage.value = `Sent permission reply: ${kind.replaceAll('_', ' ')}.`
    await loadSessionExecution(sessionId)
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

watch(activeTab, (tab) => {
  if (tab === 'plugins') {
    syncPluginLogPolling()
    return
  }
  stopPluginLogPolling()
})

onMounted(() => {
  void load()
})

onBeforeUnmount(() => {
  stopPluginLogPolling()
})
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Runtime</h1>
        <p class="page-description">Inspect runtime state, plugins, MCP, LSP, skills, providers, auth, and permission rules.</p>
      </div>
      <div class="button-row">
        <button class="button ghost" :disabled="loading" @click="load">Refresh</button>
        <button class="button primary" :disabled="loading" @click="triggerReload">Reload Runtime</button>
      </div>
    </header>

    <div v-if="actionError" class="notice">{{ actionError }}</div>
    <div v-else-if="actionMessage" class="notice">{{ actionMessage }}</div>

    <div class="button-row" style="margin-bottom: 16px; flex-wrap: wrap">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="button"
        :class="{ primary: activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </div>

    <template v-if="activeTab === 'overview'">
      <div class="grid three">
        <section v-for="card in operatorCards" :key="card.label" class="card">
          <div class="muted">{{ card.label }}</div>
          <div style="font-size: 1.5rem; font-weight: 600">{{ card.value }}</div>
        </section>
      </div>

      <div class="grid two" style="margin-top: 16px">
        <section class="card">
          <h3>Runtime Snapshot</h3>
          <div v-if="runtimeSnapshotFacts.length" class="stack">
            <div v-for="fact in runtimeSnapshotFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else class="muted">Loading runtime snapshot…</p>
        </section>

        <section class="card">
          <h3>Maintenance</h3>
          <div v-if="runtime" class="stack">
            <div><strong>Reload:</strong> {{ runtime.reload.enabled ? 'enabled' : 'disabled' }} ({{ runtime.reload.interval_secs }}s)</div>
            <div><strong>Janitor:</strong> {{ runtime.janitor.enabled ? 'enabled' : 'disabled' }} ({{ runtime.janitor.interval_secs }}s)</div>
            <div><strong>Watch Paths:</strong></div>
            <div v-if="runtime.watch_paths.length" class="list">
              <div v-for="path in runtime.watch_paths" :key="path" class="list-item mono">{{ path }}</div>
            </div>
            <div v-else class="muted">No watch paths configured.</div>
          </div>
          <p v-else class="muted">Loading maintenance state…</p>
        </section>
      </div>

      <div class="grid two" style="margin-top: 16px">
        <section class="card">
          <h3>Recent Automation</h3>
          <div v-if="runtime?.automation.recent_jobs.length" class="list">
            <div v-for="job in runtime.automation.recent_jobs" :key="job.id" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <div><strong>{{ job.kind }}</strong> <span class="muted mono">{{ job.id }}</span></div>
                  <div class="muted">session {{ job.owner_session_id ?? 'n/a' }}</div>
                  <div v-if="job.last_run" class="muted">
                    {{ job.last_run.status }} · triggered {{ job.last_run.triggered_at }}
                  </div>
                  <div v-else-if="job.next_fire_at" class="muted">next {{ job.next_fire_at }}</div>
                  <div v-if="job.last_run?.error_message" class="muted">{{ job.last_run.error_message }}</div>
                </div>
                <span class="badge">{{ job.expression || job.at || 'scheduled' }}</span>
              </div>
            </div>
          </div>
          <p v-else class="muted">No scheduled jobs visible yet.</p>
        </section>

        <section class="card">
          <h3>Provider Defaults</h3>
          <div v-if="providers.length" class="list">
            <div v-for="provider in providers" :key="provider.provider_id" class="list-item">
              <div><strong>{{ provider.provider_id }}</strong></div>
              <div class="muted">Default model: {{ provider.default_model }}</div>
              <div class="muted mono">{{ provider.default_model_ref }}</div>
              <div class="muted">
                Models:
                {{ (providerModels[provider.provider_id] || []).map((model) => formatProviderModel(model)).join(', ') || 'none' }}
              </div>
            </div>
          </div>
          <p v-else class="muted">No providers loaded.</p>
        </section>

        <section class="card">
          <h3>Session Cache</h3>
          <div v-if="sessionCacheFacts.length" class="stack">
            <div v-for="fact in sessionCacheFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong> {{ fact.value }}
            </div>
          </div>
          <p v-else class="muted">Session cache is not available.</p>
        </section>
      </div>

      <div class="grid two" style="margin-top: 16px">
        <section class="card">
          <h3>Credentials</h3>
          <div v-if="authProviders.length" class="list">
            <div v-for="provider in authProviders" :key="provider.provider_id" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <div><strong>{{ provider.provider_id }}</strong></div>
                  <div v-for="fact in buildAuthProviderFacts(provider)" :key="fact.label" class="muted">
                    <strong>{{ fact.label }}:</strong>
                    <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
                  </div>
                </div>
                <span class="badge">{{ provider.credential_type || 'unknown' }}</span>
              </div>

              <div class="field" style="margin-top: 12px">
                <label class="label" :for="`api-key-${provider.provider_id}`">API Key</label>
                <input
                  :id="`api-key-${provider.provider_id}`"
                  v-model="drafts[provider.provider_id]"
                  class="input mono"
                  type="password"
                  placeholder="sk-..."
                />
              </div>

              <div class="button-row" style="margin-top: 12px">
                <button class="button primary" @click="saveApiKey(provider.provider_id)">Save API Key</button>
                <button class="button" @click="refreshCredential(provider.provider_id)">Refresh</button>
                <button class="button danger" @click="clearCredential(provider.provider_id)">Delete</button>
              </div>
            </div>
          </div>
          <p v-else class="muted">No auth-capable providers were exposed by the runtime.</p>
        </section>

        <section class="card">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <h3>Permission Rules</h3>
              <p class="muted">Persist allow / ask / deny decisions as structured tool/path rules with scope and source metadata.</p>
            </div>
            <button class="button ghost" :disabled="loading" @click="load">Refresh</button>
          </div>

          <div class="field">
            <label class="label" for="permission-search">Search</label>
            <input
              id="permission-search"
              v-model="permissionSearch"
              class="input mono"
              placeholder="Bash:ls"
              @keyup.enter="load"
            />
          </div>

          <div class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="permission-status-filter">Status</label>
              <select id="permission-status-filter" v-model="permissionStatusFilter" class="select">
                <option value="active">active</option>
                <option value="revoked">revoked</option>
                <option value="all">all</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-scope-filter">Scope</label>
              <select id="permission-scope-filter" v-model="permissionScopeFilter" class="select">
                <option value="all">all</option>
                <option value="workspace">workspace</option>
                <option value="session">session</option>
                <option value="global">global</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-mode-filter">Mode</label>
              <select id="permission-mode-filter" v-model="permissionModeFilter" class="select">
                <option value="all">all</option>
                <option value="allow">allow</option>
                <option value="ask">ask</option>
                <option value="deny">deny</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-subject-filter">Subject</label>
              <select id="permission-subject-filter" v-model="permissionSubjectFilter" class="select">
                <option value="all">all</option>
                <option value="builtin_tool">builtin_tool</option>
                <option value="path_access">path_access</option>
              </select>
            </div>
          </div>

          <div class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="permission-subject-kind">Subject</label>
              <select id="permission-subject-kind" v-model="permissionDraft.subjectKind" class="select">
                <option value="builtin_tool">builtin_tool</option>
                <option value="path_access">path_access</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-mode">Mode</label>
              <select id="permission-mode" v-model="permissionDraft.mode" class="select">
                <option value="allow">allow</option>
                <option value="ask">ask</option>
                <option value="deny">deny</option>
              </select>
            </div>
          </div>

          <div v-if="permissionDraft.subjectKind === 'builtin_tool'" class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="permission-tool-name">Tool Name</label>
              <input
                id="permission-tool-name"
                v-model="permissionDraft.toolName"
                class="input mono"
                placeholder="bash"
              />
            </div>
            <div class="field">
              <label class="label" for="permission-qualifier">Qualifier</label>
              <input
                id="permission-qualifier"
                v-model="permissionDraft.qualifier"
                class="input mono"
                placeholder="git status *"
              />
            </div>
          </div>

          <div v-else class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="permission-path-access-kind">Path Access</label>
              <select id="permission-path-access-kind" v-model="permissionDraft.pathAccessKind" class="select">
                <option value="read">read</option>
                <option value="write">write</option>
                <option value="external_directory">external_directory</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-target-path">Target Path</label>
              <input
                id="permission-target-path"
                v-model="permissionDraft.targetPath"
                class="input mono"
                placeholder="src/**"
              />
            </div>
            <div class="field" style="grid-column: 1 / -1;">
              <label class="label" for="permission-workspace-root">Workspace Root Override</label>
              <input
                id="permission-workspace-root"
                v-model="permissionDraft.workspaceRoot"
                class="input mono"
                placeholder="optional workspace root override"
              />
            </div>
          </div>

          <div class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="permission-scope">Scope</label>
              <select id="permission-scope" v-model="permissionDraft.scope" class="select">
                <option value="workspace">workspace</option>
                <option value="session">session</option>
                <option value="global">global</option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="permission-session-id">Session ID</label>
              <input
                id="permission-session-id"
                v-model="permissionDraft.sessionId"
                class="input mono"
                placeholder="required for session scope"
                :disabled="permissionDraft.scope !== 'session'"
              />
            </div>
          </div>

          <div class="button-row" style="margin-top: 12px">
            <button class="button primary" @click="savePermissionRule">
              {{ editingPermissionRuleId ? 'Update Rule' : 'Create Rule' }}
            </button>
            <button class="button" @click="resetPermissionDraft">Reset</button>
          </div>

          <div v-if="filteredPermissionRules.length" class="list" style="margin-top: 12px">
            <div v-for="rule in filteredPermissionRules" :key="rule.id" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <strong class="mono">{{ permissionRuleLabel(rule) }}</strong>
                  <div class="muted">{{ permissionRulePreview(rule) }}</div>
                  <div class="muted">updated {{ rule.updated_at }}</div>
                  <div class="muted mono">{{ permissionRuleFacts(rule).join(' · ') }}</div>
                </div>
                <div class="button-row">
                  <span class="badge">{{ rule.mode }}</span>
                  <span class="badge">{{ rule.revoked_at ? 'revoked' : 'active' }}</span>
                </div>
              </div>
              <div class="button-row" style="margin-top: 10px">
                <button class="button" :disabled="Boolean(rule.revoked_at)" @click="editPermissionRule(rule)">Edit</button>
                <button class="button danger" :disabled="Boolean(rule.revoked_at)" @click="revokePermissionRuleAction(rule)">
                  Revoke
                </button>
              </div>
            </div>
          </div>
          <p v-else class="muted" style="margin-top: 12px">No permission rules matched the current filters.</p>
        </section>
      </div>
    </template>

    <template v-else-if="activeTab === 'workflow'">
      <div class="grid two">
        <section class="card">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <h3>Workflow Inspector</h3>
              <p class="muted">Observe real session execution state without leaving the runtime page.</p>
            </div>
            <button class="button ghost" :disabled="!selectedSessionId" @click="openSelectedSessionInChat">
              Open in Chat
            </button>
          </div>

          <div class="grid two" style="margin-top: 12px">
            <div class="field">
              <label class="label" for="workflow-workspace">Workspace</label>
              <select
                id="workflow-workspace"
                :value="selectedWorkspaceId ?? ''"
                class="select"
                @change="selectWorkspace(Number(($event.target as HTMLSelectElement).value))"
              >
                <option v-for="workspace in workspaces" :key="workspace.id" :value="workspace.id">
                  #{{ workspace.id }} · {{ workspace.path }}
                </option>
              </select>
            </div>
            <div class="field">
              <label class="label" for="workflow-session">Session</label>
              <select
                id="workflow-session"
                :value="selectedSessionId ?? ''"
                class="select"
                @change="selectSession(Number(($event.target as HTMLSelectElement).value))"
              >
                <option v-for="session in sessions" :key="session.id" :value="session.id">
                  #{{ session.id }} · {{ session.title }}
                </option>
              </select>
            </div>
          </div>

          <div v-if="executionFacts.length" class="stack" style="margin-top: 12px">
            <div v-for="fact in executionFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else-if="workflowLoading" class="muted" style="margin-top: 12px">Loading execution state…</p>
          <p v-else class="muted" style="margin-top: 12px">Select a session to inspect workflow execution state.</p>
        </section>

        <section class="card">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <h3>Pending Permissions</h3>
              <p class="muted">Approve or deny pending requests directly from the runtime workflow inspector.</p>
            </div>
            <span class="badge">{{ sessionExecution?.pending_permission_requests.length || 0 }}</span>
          </div>
          <div v-if="sessionExecution?.pending_permission_requests?.length" class="list">
            <div
              v-for="request in sessionExecution.pending_permission_requests"
              :key="request.request_id"
              class="list-item"
            >
              <div>
                <strong>{{ permissionActionView(request.action).title }}</strong>
              </div>
              <div class="muted mono">request_id={{ request.request_id }}</div>
              <div class="muted">{{ request.reason }}</div>
              <div class="muted">risk={{ permissionRiskLabel(request.action) }}</div>
              <div v-if="request.explanation" class="muted">{{ request.explanation }}</div>
              <div
                v-if="permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).summary"
                class="muted"
              >
                {{ permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).summary }}
              </div>
              <div
                v-if="permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).details.length"
                class="muted mono"
              >
                {{ permissionExplainability({ source: request.source, scope: request.scope, operator: request.operator }).details.join(' · ') }}
              </div>
              <div class="muted mono">{{ permissionActionView(request.action).details.join(' · ') }}</div>
              <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
                <button class="button primary" @click="approvePermission(request.request_id, 'allow_once')">
                  Allow Once
                </button>
                <button class="button" @click="approvePermission(request.request_id, 'allow_always', 'session')">
                  Allow Always (Session)
                </button>
                <button class="button" @click="approvePermission(request.request_id, 'allow_always', 'workspace')">
                  Allow Always (Workspace)
                </button>
                <button class="button" @click="approvePermission(request.request_id, 'allow_always', 'global')">
                  Allow Always (Global)
                </button>
                <button class="button danger" @click="approvePermission(request.request_id, 'deny_once')">
                  Deny Once
                </button>
                <button class="button danger" @click="approvePermission(request.request_id, 'deny_always', 'session')">
                  Deny Always (Session)
                </button>
                <button class="button danger" @click="approvePermission(request.request_id, 'deny_always', 'workspace')">
                  Deny Always (Workspace)
                </button>
                <button class="button danger" @click="approvePermission(request.request_id, 'deny_always', 'global')">
                  Deny Always (Global)
                </button>
              </div>
              <div class="muted">
                once={{ permissionReplyPreview() }} · session={{ permissionReplyPreview('session') }} · workspace={{ permissionReplyPreview('workspace') }} · global={{ permissionReplyPreview('global') }}
              </div>
            </div>
          </div>
          <p v-else-if="workflowLoading" class="muted">Loading pending permissions…</p>
          <p v-else class="muted">No pending permission requests for the selected session.</p>
        </section>

        <section class="card">
          <h3>Recent Timeline</h3>
          <div v-if="timelineSummaries.length" class="list">
            <div v-for="event in timelineSummaries" :key="event.key" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <div><strong>{{ event.kind }}</strong></div>
                  <div class="muted">{{ event.summary }}</div>
                  <div class="muted">{{ event.sessionId }}</div>
                </div>
                <span class="badge">{{ event.timestamp }}</span>
              </div>
            </div>
          </div>
          <p v-else-if="workflowLoading" class="muted">Loading session timeline…</p>
          <p v-else class="muted">No timeline events loaded yet.</p>
        </section>
      </div>
    </template>

    <template v-else-if="activeTab === 'plugins'">
      <div class="grid two">
        <section class="card">
          <h3>Plugins</h3>
          <div v-if="plugins.length" class="list">
            <button
              v-for="plugin in plugins"
              :key="plugin.plugin_id"
              class="list-item"
              style="width: 100%; text-align: left"
              @click="loadPluginDetails(plugin.plugin_id)"
            >
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <div><strong>{{ plugin.plugin_id }}</strong></div>
                  <div class="muted">{{ plugin.kind }} · {{ plugin.state }}</div>
                  <div v-if="plugin.last_error" class="muted">{{ plugin.last_error }}</div>
                </div>
                <span class="badge">restarts {{ plugin.restart_count }}</span>
              </div>
            </button>
          </div>
          <p v-else class="muted">No configured plugins.</p>
        </section>

        <section class="card">
          <h3>Plugin Detail</h3>
          <div v-if="selectedPlugin" class="stack">
            <div><strong>ID:</strong> {{ selectedPlugin.status.plugin_id }}</div>
            <div><strong>Kind:</strong> {{ selectedPlugin.status.kind }}</div>
            <div><strong>State:</strong> {{ selectedPlugin.status.state }}</div>
            <div><strong>PID:</strong> {{ selectedPlugin.status.pid ?? 'n/a' }}</div>
            <div><strong>Last Exit:</strong> {{ selectedPlugin.status.last_exit_code ?? 'n/a' }}</div>
            <div><strong>Last Restart:</strong> {{ selectedPlugin.status.last_restart_at_ms ?? 'n/a' }}</div>
            <div><strong>Manifest:</strong></div>
            <pre class="mono" style="white-space: pre-wrap">{{ JSON.stringify(selectedPlugin.manifest ?? {}, null, 2) }}</pre>
            <div><strong>Recent Logs:</strong></div>
            <div v-if="pluginLogs.length" class="list">
              <div v-for="entry in pluginLogs" :key="entry.seq" class="list-item">
                <div class="page-header" style="align-items: flex-start">
                  <strong>#{{ entry.seq }}</strong>
                  <span class="badge">{{ entry.level }}</span>
                </div>
                <div class="muted">{{ entry.target || 'plugin' }} · {{ entry.timestamp_ms }}</div>
                <div class="muted">{{ entry.message }}</div>
              </div>
            </div>
            <div v-else class="muted">No retained logs.</div>
          </div>
          <p v-else-if="pluginLoading" class="muted">Loading plugin detail…</p>
          <p v-else class="muted">Select a plugin to inspect.</p>
        </section>
      </div>
    </template>

    <template v-else-if="activeTab === 'mcp'">
      <section class="card">
        <h3>MCP Servers</h3>
        <div v-if="runtime" class="stack">
          <div><strong>Server Count:</strong> {{ runtime.operator.mcp.server_count }}</div>
          <div><strong>Tool Count:</strong> {{ runtime.operator.mcp.tool_count }}</div>
          <div v-if="runtime.operator.mcp.servers.length" class="list">
            <div v-for="server in runtime.operator.mcp.servers" :key="server.name" class="list-item">
              <div><strong>{{ server.name }}</strong></div>
              <div class="muted">tools {{ server.tool_count }}</div>
            </div>
          </div>
          <div v-else class="muted">No MCP servers connected.</div>
        </div>
      </section>
    </template>

    <template v-else-if="activeTab === 'lsp'">
      <section class="card">
        <h3>LSP Fleet</h3>
        <div v-if="runtime" class="stack">
          <div><strong>Server Count:</strong> {{ runtime.operator.lsp.server_count }}</div>
          <div><strong>Diagnostics:</strong> {{ runtime.operator.lsp.diagnostics_count }}</div>
          <div><strong>Files With Diagnostics:</strong> {{ runtime.operator.lsp.files_with_diagnostics }}</div>
          <div v-if="runtime.operator.lsp.servers.length" class="list">
            <div v-for="server in runtime.operator.lsp.servers" :key="server.name" class="list-item">
              <div><strong>{{ server.name }}</strong></div>
              <div class="muted mono">{{ server.command }}</div>
              <div class="muted">extensions: {{ server.file_extensions.join(', ') || 'all' }}</div>
              <div class="muted">root markers: {{ server.root_markers.join(', ') || 'workspace root' }}</div>
            </div>
          </div>
          <div v-else class="muted">No LSP servers configured.</div>
        </div>
      </section>
    </template>

    <template v-else-if="activeTab === 'skills'">
      <div class="grid two">
        <section class="card">
          <h3>Skills</h3>
          <div v-if="discoveredSkills.length" class="list">
            <div v-for="skill in discoveredSkills" :key="skill.name" class="list-item">
              <div><strong>{{ skill.name }}</strong></div>
              <div class="muted">{{ skill.description || 'No description' }}</div>
              <div v-if="skill.aliases.length" class="muted">aliases: {{ skill.aliases.join(', ') }}</div>
              <div v-if="skill.source_path" class="muted mono">{{ skill.source_path }}</div>
            </div>
          </div>
          <p v-else class="muted">No skills discovered.</p>
        </section>

        <section class="card">
          <h3>Commands</h3>
          <div v-if="skillCommands.length" class="list">
            <div v-for="skill in skillCommands" :key="skill.name" class="list-item">
              <div><strong>{{ skill.name }}</strong></div>
              <div class="muted">{{ skill.description || 'No description' }}</div>
              <div v-if="skill.aliases.length" class="muted">aliases: {{ skill.aliases.join(', ') }}</div>
              <div v-if="skill.source_path" class="muted mono">{{ skill.source_path }}</div>
            </div>
          </div>
          <p v-else class="muted">No commands discovered.</p>
        </section>
      </div>
    </template>

    <template v-else-if="activeTab === 'operator'">
      <div class="grid three">
        <section class="card">
          <h3>Runtime</h3>
          <div v-if="runtime" class="stack">
            <div><strong>Config Found:</strong> {{ runtime.config_found ? 'yes' : 'no' }}</div>
            <div><strong>Session Runtime:</strong> {{ runtime.session_runtime_available ? 'enabled' : 'disabled' }}</div>
            <div><strong>Watch Paths:</strong> {{ runtime.watch_paths.length }}</div>
          </div>
        </section>
        <section class="card">
          <h3>MCP</h3>
          <div v-if="runtime" class="stack">
            <div><strong>Servers:</strong> {{ runtime.operator.mcp.server_count }}</div>
            <div><strong>Tools:</strong> {{ runtime.operator.mcp.tool_count }}</div>
          </div>
        </section>
        <section class="card">
          <h3>LSP + Skills</h3>
          <div v-if="runtime" class="stack">
            <div><strong>LSP Servers:</strong> {{ runtime.operator.lsp.server_count }}</div>
            <div><strong>Diagnostics:</strong> {{ runtime.operator.lsp.diagnostics_count }}</div>
            <div><strong>Skills:</strong> {{ runtime.operator.skills.skill_count }}</div>
            <div><strong>Commands:</strong> {{ runtime.operator.skills.command_count }}</div>
          </div>
        </section>
      </div>
    </template>
  </section>
</template>
