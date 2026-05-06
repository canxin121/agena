<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import {
  createPermissionRule,
  deletePermissionRule,
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
const permissionDraft = reactive<{ actionKey: string; mode: PermissionMode }>({
  actionKey: '',
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
  permissionDraft.actionKey = ''
  permissionDraft.mode = 'ask'
  editingPermissionRuleId.value = null
}

function editPermissionRule(rule: PermissionRuleResource) {
  permissionDraft.actionKey = rule.action_key
  permissionDraft.mode = rule.mode
  editingPermissionRuleId.value = rule.id
}

async function savePermissionRule() {
  const actionKey = permissionDraft.actionKey.trim()
  if (!actionKey) return
  actionMessage.value = ''
  actionError.value = ''
  try {
    if (editingPermissionRuleId.value) {
      await updatePermissionRule({
        id: editingPermissionRuleId.value,
        actionKey,
        mode: permissionDraft.mode,
      })
      actionMessage.value = `Updated permission rule for ${actionKey}.`
    } else {
      await createPermissionRule({ actionKey, mode: permissionDraft.mode })
      actionMessage.value = `Created permission rule for ${actionKey}.`
    }
    resetPermissionDraft()
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function removePermissionRule(rule: PermissionRuleResource) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await deletePermissionRule(rule.id)
    actionMessage.value = `Deleted permission rule for ${rule.action_key}.`
    if (editingPermissionRuleId.value === rule.id) resetPermissionDraft()
    await load()
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
              <p class="muted">Persist allow / ask / deny decisions by action key.</p>
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
              <label class="label" for="permission-action-key">Action Key</label>
              <input
                id="permission-action-key"
                v-model="permissionDraft.actionKey"
                class="input mono"
                placeholder="Tool:action"
              />
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

          <div class="button-row" style="margin-top: 12px">
            <button class="button primary" @click="savePermissionRule">
              {{ editingPermissionRuleId ? 'Update Rule' : 'Create Rule' }}
            </button>
            <button class="button" @click="resetPermissionDraft">Reset</button>
          </div>

          <div v-if="permissionRules.length" class="list" style="margin-top: 12px">
            <div v-for="rule in permissionRules" :key="rule.id" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <strong class="mono">{{ rule.action_key }}</strong>
                  <div class="muted">updated {{ rule.updated_at }}</div>
                </div>
                <span class="badge">{{ rule.mode }}</span>
              </div>
              <div class="button-row" style="margin-top: 10px">
                <button class="button" @click="editPermissionRule(rule)">Edit</button>
                <button class="button danger" @click="removePermissionRule(rule)">Delete</button>
              </div>
            </div>
          </div>
          <p v-else class="muted" style="margin-top: 12px">No permission rules found.</p>
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
