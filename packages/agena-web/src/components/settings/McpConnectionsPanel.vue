<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router'
import { RiAddLine, RiArrowLeftLine, RiDeleteBinLine, RiExternalLinkLine, RiPlugLine, RiRefreshLine, RiUpload2Line } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsSectionWorkbench from './workbench/SettingsSectionWorkbench.vue'
import { buildSettingsSubpages, SETTINGS_DEFAULT_SUBPAGE } from './settingsNavigationCatalog'
import McpCapabilitiesPanel from './mcp/McpCapabilitiesPanel.vue'
import McpConfigEditor from './mcp/McpConfigEditor.vue'
import McpImportDialog from './mcp/McpImportDialog.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction } from '@/lib/appConfirm'
import { useToastsStore } from '@/stores/toasts'
import { mcpConnectionsApi as api, type McpCatalog, type McpConnectionDetail, type McpImportResult, type McpMutationResult, type McpScope, type McpTestResult } from '@/lib/mcpConnectionsApi'
import { MCP_AUTH_STORAGE_KEY, cloneMcpConfig, filterMcpConnections, mcpCallbackMatches, mcpConfigProblems, mcpDraftFromDetail, mcpEndpoint, mcpRecord, newMcpConfig, preservedMcpSecrets, readMcpPendingAuth, type McpDraft, type McpPendingAuth } from '@/lib/mcpConnections'

defineProps<{ activePage: string }>()
const route = useRoute()
const router = useRouter()
const toasts = useToastsStore()
const fieldId = useId()
const pages = computed(() => buildSettingsSubpages('mcp-connections'))
const catalog = ref<McpCatalog | null>(null)
const selectedId = ref(typeof route.query.connection === 'string' ? route.query.connection : '')
const detail = ref<McpConnectionDetail | null>(null)
const draft = ref<McpDraft | null>(null)
const savedDraft = ref('')
const draftRevision = ref('')
const editorValid = ref(true)
const editorEpoch = ref(0)
const loading = ref(false)
const detailLoading = ref(false)
const action = ref('')
const error = ref('')
const actionError = ref('')
const notice = ref('')
const query = ref('')
const scopeFilter = ref('all')
const statusFilter = ref('all')
const showDetail = ref(false)
const tab = ref('configuration')
const testResult = ref<McpTestResult | null>(null)
const createOpen = ref(false)
const createName = ref('')
const createScope = ref<McpScope>('workspace')
const createDraft = ref<McpDraft>({ config: newMcpConfig(), preserveSecrets: [] })
const createValid = ref(true)
const createError = ref('')
const importOpen = ref(false)
const importDirty = ref(false)
const bridgeScope = ref<McpScope>('workspace')
const storedToken = ref('')
const pendingAuth = ref<McpPendingAuth | null>(null)
const authorizationUrl = ref('')
const callbackUrl = ref('')
let disposed = false
let listGeneration = 0
let detailGeneration = 0
let listController: AbortController | undefined
let detailController: AbortController | undefined
let refreshTimer: ReturnType<typeof setInterval> | undefined
let reloadTimer: ReturnType<typeof setTimeout> | undefined
const busy = computed(() => Boolean(action.value))
const dirty = computed(() => Boolean(draft.value) && (!editorValid.value || JSON.stringify(draft.value) !== savedDraft.value))
const createDirty = computed(() => !createValid.value || Boolean(createName.value) || JSON.stringify(createDraft.value.config) !== JSON.stringify(newMcpConfig()))
const hasUnsaved = computed(() => dirty.value || (createOpen.value && createDirty.value) || importDirty.value)
const ready = computed(() => detail.value?.id === selectedId.value && !detailLoading.value)
const items = computed(() => filterMcpConnections(catalog.value?.connections || [], query.value, scopeFilter.value, statusFilter.value))
const connectedCount = computed(() => catalog.value?.connections.filter((connection) => connection.state === 'connected').length || 0)
const draftProblems = computed(() => draft.value ? mcpConfigProblems(draft.value) : [])
const createProblems = computed(() => mcpConfigProblems(createDraft.value, true))
const changedElsewhere = computed(() => Boolean(dirty.value && detail.value && detail.value.revision !== draftRevision.value))
const authKind = computed(() => { const kind = String(mcpRecord(detail.value?.config.auth).kind || 'none'); return kind === 'oauth' ? 'o_auth' : kind })
const scopeOptions = computed(() => [{ value: 'workspace', label: st('Workspace') }, { value: 'global', label: st('Global') }])
const filterScopes = computed(() => [{ value: 'all', label: st('All scopes') }, ...scopeOptions.value])
const statusOptions = computed(() => [
  { value: 'all', label: st('All statuses') }, { value: 'connected', label: st('Connected') },
  { value: 'attention', label: st('Needs attention') }, { value: 'disabled', label: st('Disabled') },
  { value: 'shadowed', label: st('Overridden') },
])
const tabs = computed(() => [
  { id: 'configuration', label: st('Configuration') }, { id: 'tools', label: st('Tools') },
  { id: 'resources', label: st('Resources') }, { id: 'prompts', label: st('Prompts') },
])
const labels = computed<Record<string, string>>(() => ({
  connected: st('Connected'), disconnected: st('Disconnected'), disabled: st('Disabled'), shadowed: st('Overridden'),
  bridge_disabled: st('MCP bridge disabled'), invalid: st('Invalid configuration'), authentication_required: st('Sign-in required'),
  pending_reload: st('Pending reload'),
}))
function stateLabel(state: string) { return labels.value[state] || state }
function stateClass(state: string) {
  return state === 'connected' ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : ['invalid', 'authentication_required', 'disconnected', 'pending_reload'].includes(state) ? 'border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-200' : 'border-border/70 bg-muted/40 text-muted-foreground'
}
const scopeLabel = (scope: string) => scope === 'global' ? st('Global') : st('Workspace')
const message = (reason: unknown) => reason instanceof Error ? reason.message : String(reason)
function acceptDetail(next: McpConnectionDetail, reset = false) {
  const replacing = detail.value?.id !== next.id
  detail.value = next
  if (reset || replacing || !dirty.value) {
    draft.value = mcpDraftFromDetail(next)
    savedDraft.value = JSON.stringify(draft.value)
    draftRevision.value = next.revision
    editorValid.value = true
    if (reset || replacing) editorEpoch.value++
  }
}
async function loadDetail(id: string, reset = false) {
  const request = ++detailGeneration
  detailController?.abort()
  const controller = new AbortController()
  detailController = controller
  if (reset || detail.value?.id !== id || !draft.value) detailLoading.value = true
  try {
    const next = await api.get(id, controller.signal)
    if (disposed || request !== detailGeneration || selectedId.value !== id) return
    acceptDetail(next, reset)
  } catch (reason) {
    if (!disposed && request === detailGeneration && !controller.signal.aborted) actionError.value = message(reason)
  } finally { if (request === detailGeneration) detailLoading.value = false }
}
async function refresh() {
  if (busy.value) return
  const request = ++listGeneration
  listController?.abort()
  const controller = new AbortController()
  listController = controller
  loading.value = true
  error.value = ''
  try {
    const next = await api.list(controller.signal)
    if (disposed || request !== listGeneration) return
    catalog.value = next
    if (selectedId.value && dirty.value && !next.connections.some((connection) => connection.id === selectedId.value)) {
      actionError.value = st('This connection was removed elsewhere. Your draft is still available.')
      return
    }
    const id = next.connections.some((connection) => connection.id === selectedId.value) ? selectedId.value : next.connections[0]?.id || ''
    selectedId.value = id
    if (id) await loadDetail(id)
    else { detail.value = null; draft.value = null; savedDraft.value = ''; showDetail.value = false }
  } catch (reason) { if (!disposed && request === listGeneration && !controller.signal.aborted) error.value = message(reason) }
  finally { if (request === listGeneration) loading.value = false }
}
async function canDiscard() {
  if (busy.value) return false
  if (!hasUnsaved.value) return true
  return await confirmAction({ title: st('Discard unsaved changes?'), description: st('You have unsaved changes.'), confirmText: st('Discard changes'), cancelText: st('Keep editing') })
}
async function select(id: string) {
  if (id === selectedId.value) { showDetail.value = true; return }
  if (!(await canDiscard())) return
  if (id !== selectedId.value) { draft.value = null; savedDraft.value = ''; storedToken.value = ''; testResult.value = null; actionError.value = ''; tab.value = 'configuration' }
  selectedId.value = id
  showDetail.value = true
  await loadDetail(id, true)
}
async function run(key: string, execute: () => Promise<void>) {
  if (busy.value) return false
  action.value = key
  actionError.value = ''
  try { await execute(); return true }
  catch (reason) { if (!disposed) actionError.value = message(reason); return false }
  finally { if (!disposed) action.value = '' }
}
function scheduleReloadRefresh(taskId: string | null) {
  if (!taskId) return
  if (reloadTimer) clearTimeout(reloadTimer)
  let attempts = 0
  const tick = async () => {
    if (disposed) return
    await refresh()
    if (++attempts < 8) reloadTimer = setTimeout(tick, 1500)
  }
  reloadTimer = setTimeout(tick, 1000)
}
function mutationNotice(result: McpMutationResult | McpImportResult) {
  if (disposed) return
  notice.value = result.warning || (result.reload_task_id ? st('Configuration saved. The runtime is reloading.') : '')
  scheduleReloadRefresh(result.reload_task_id)
}
async function save() {
  if (!ready.value || !draft.value || !dirty.value || !editorValid.value || draftProblems.value.length) return
  const id = selectedId.value
  const snapshot = cloneMcpConfig(draft.value)
  const revision = draftRevision.value
  const succeeded = await run('save', async () => {
    const result = await api.update(id, snapshot.config, preservedMcpSecrets(snapshot), revision)
    if (!result.saved) throw new Error(result.warning || st('The connection was not saved.'))
    if (!disposed && selectedId.value === id) {
      if (result.connection) acceptDetail(result.connection, true)
      else { savedDraft.value = JSON.stringify(snapshot); await loadDetail(id, true) }
      testResult.value = null
      mutationNotice(result)
    }
    toasts.push('success', st('MCP connection saved'))
  })
  if (succeeded) await refresh()
}
async function discard() {
  if (!detail.value || !(await canDiscard())) return
  actionError.value = ''
  await loadDetail(selectedId.value, true)
}
async function toggleEnabled() {
  if (!ready.value || !detail.value || dirty.value) return
  const current = detail.value
  const succeeded = await run('enabled', async () => {
    const result = await api.setEnabled(current.id, !current.enabled, current.revision)
    if (result.connection && selectedId.value === current.id) acceptDetail(result.connection, true)
    mutationNotice(result)
  })
  if (succeeded) await refresh()
}
async function remove() {
  if (!ready.value || !detail.value || busy.value || !(await canDiscard())) return
  const current = detail.value
  if (!(await confirmAction({ title: st('Remove MCP connection?'), description: current.name + ' · ' + scopeLabel(current.scope), confirmText: st('Remove connection'), cancelText: st('Cancel'), variant: 'destructive' }))) return
  const succeeded = await run('remove', async () => {
    const result = await api.remove(current.id, current.revision)
    if (selectedId.value === current.id) { selectedId.value = ''; detail.value = null; draft.value = null; savedDraft.value = ''; showDetail.value = false }
    mutationNotice(result)
    toasts.push('success', st('MCP connection removed'))
  })
  if (succeeded) await refresh()
}
async function test() {
  if (!ready.value || !detail.value || dirty.value) return
  const id = detail.value.id
  testResult.value = null
  await run('test', async () => { const result = await api.test(id); if (!disposed && selectedId.value === id) testResult.value = result })
}
async function reconnect() {
  if (!ready.value || !detail.value || dirty.value || detail.value.state === 'pending_reload') return
  const id = detail.value.id
  const succeeded = await run('reconnect', async () => { const next = await api.reconnect(id); if (selectedId.value === id) acceptDetail(next, true) })
  if (succeeded) await refresh()
}
async function setBridge(enabled: boolean) {
  if (busy.value) return
  const scope = bridgeScope.value
  const succeeded = await run('bridge', async () => { catalog.value = await api.setBridgeEnabled(enabled, scope) })
  if (succeeded) { notice.value = st('MCP bridge configuration saved.'); await refresh() }
}
async function openCreate() {
  if (!(await canDiscard())) return
  if (dirty.value && detail.value) acceptDetail(detail.value, true)
  createName.value = ''; createScope.value = 'workspace'; createDraft.value = { config: newMcpConfig(), preserveSecrets: [] }; createError.value = ''; createValid.value = true; createOpen.value = true
}
async function openImport() {
  if (!(await canDiscard())) return
  if (dirty.value && detail.value) acceptDetail(detail.value, true)
  importOpen.value = true
}
async function closeCreate() {
  if (busy.value) return
  if (createDirty.value && !(await confirmAction({ title: st('Discard unsaved changes?'), description: st('You have unsaved changes.'), confirmText: st('Discard changes'), cancelText: st('Keep editing') }))) return
  createOpen.value = false
  createName.value = ''; createDraft.value = { config: newMcpConfig(), preserveSecrets: [] }
}
async function create() {
  if (busy.value || !createName.value.trim() || !createValid.value || createProblems.value.length) return
  createError.value = ''
  action.value = 'create'
  try {
    const result = await api.create(createName.value.trim(), createScope.value, cloneMcpConfig(createDraft.value.config))
    if (!result.saved) throw new Error(result.warning || st('The connection was not saved.'))
    if (disposed) return
    createOpen.value = false
    createName.value = ''; createDraft.value = { config: newMcpConfig(), preserveSecrets: [] }
    if (result.connection) { selectedId.value = result.connection.id; acceptDetail(result.connection, true); showDetail.value = true; tab.value = 'configuration' }
    mutationNotice(result)
    toasts.push('success', st('MCP connection created'))
  } catch (reason) { if (!disposed) createError.value = message(reason) }
  finally { if (!disposed) action.value = '' }
  if (!createOpen.value) await refresh()
}
async function imported(result: McpImportResult) {
  mutationNotice(result)
  const first = result.installed[0]
  if (first) { selectedId.value = first.id; draft.value = null; savedDraft.value = ''; showDetail.value = true; tab.value = 'configuration' }
  toasts.push('success', st('MCP connections imported'))
  await refresh()
}
function clearPendingAuth() {
  pendingAuth.value = null; authorizationUrl.value = ''; callbackUrl.value = ''
  try { sessionStorage.removeItem(MCP_AUTH_STORAGE_KEY) } catch {}
}
async function startAuth(manual = false) {
  if (!ready.value || !detail.value || dirty.value) return
  const id = detail.value.id
  await run('auth-start', async () => {
    if (pendingAuth.value) await api.logout(pendingAuth.value.connection_id, pendingAuth.value.flow_id).catch(() => undefined)
    clearPendingAuth()
    const flow = await api.startAuth(id, location.origin + '/settings/mcp-connections')
    const url = new URL(flow.authorization_url)
    const state = url.searchParams.get('state')
    if (!['http:', 'https:'].includes(url.protocol) || !state) throw new Error(st('The service returned an invalid authorization URL.'))
    const pending = { flow_id: flow.flow_id, connection_id: id, state, expires_at: flow.expires_at }
    pendingAuth.value = pending
    authorizationUrl.value = url.href
    let stored = false
    try { sessionStorage.setItem(MCP_AUTH_STORAGE_KEY, JSON.stringify(pending)); stored = true } catch {}
    if (!manual && stored) location.assign(url.href)
    else if (!stored) notice.value = st('Browser storage is unavailable. Keep this page open and use manual authorization.')
  })
}
async function completeAuth(input = callbackUrl.value) {
  const pending = pendingAuth.value
  if (!pending || busy.value) return
  if (!mcpCallbackMatches(pending, input)) { actionError.value = st('The callback URL does not match this sign-in request.'); return }
  const succeeded = await run('auth-complete', async () => {
    const next = await api.completeAuth(pending.connection_id, pending.flow_id, input)
    if (!disposed && selectedId.value === next.id) acceptDetail(next, true)
    clearPendingAuth()
    toasts.push('success', st('MCP sign-in completed'))
  })
  if (succeeded) await refresh()
}
async function logout(cancel = false) {
  if (!detail.value || dirty.value || busy.value) return
  const id = detail.value.id
  if (!cancel && !(await confirmAction({ title: st('Sign out of this MCP service?'), confirmText: st('Sign out'), cancelText: st('Cancel') }))) return
  const succeeded = await run('auth-logout', async () => {
    const next = await api.logout(id, cancel ? pendingAuth.value?.flow_id : undefined)
    if (selectedId.value === id) acceptDetail(next, true)
    clearPendingAuth()
  })
  if (succeeded) await refresh()
}
async function credential(removeToken = false) {
  if (!ready.value || !detail.value || dirty.value || (!removeToken && !storedToken.value)) return
  if (removeToken && !(await confirmAction({ title: st('Remove stored token?'), confirmText: st('Remove token'), cancelText: st('Cancel'), variant: 'destructive' }))) return
  const id = detail.value.id
  const token = storedToken.value
  const succeeded = await run('credential', async () => {
    const next = removeToken ? await api.removeCredential(id) : await api.setCredential(id, token)
    if (selectedId.value === id) { acceptDetail(next, true); storedToken.value = '' }
    toasts.push('success', st('MCP credential updated'))
  })
  if (succeeded) await refresh()
}
async function resumeAuth() {
  const current = new URL(location.href)
  if (!current.searchParams.has('code') && !current.searchParams.has('error')) return
  const pending = pendingAuth.value
  const matches = pending && current.searchParams.get('state') === pending.state
  if (pending && matches && current.searchParams.has('error')) {
    await run('auth-cancel', async () => { await api.logout(pending.connection_id, pending.flow_id); clearPendingAuth() })
    actionError.value = st('Authorization was cancelled or denied.')
  } else if (pending && matches) {
    selectedId.value = pending.connection_id
    showDetail.value = true
    await loadDetail(pending.connection_id, true)
    callbackUrl.value = current.href
    await completeAuth(current.href)
  } else actionError.value = st('No matching sign-in request was found. Start sign-in again.')
  const query = { ...route.query }
  for (const key of ['code', 'state', 'error', 'error_description', 'iss']) delete query[key]
  await router.replace({ path: route.path, query, hash: route.hash })
}
function beforeUnload(event: BeforeUnloadEvent) { if (hasUnsaved.value) { event.preventDefault(); event.returnValue = '' } }
function onKey(event: KeyboardEvent) {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
    if (createOpen.value) { event.preventDefault(); void create() }
    else if (dirty.value && !importOpen.value) { event.preventDefault(); void save() }
  }
}
onBeforeRouteLeave(async () => await canDiscard())
onBeforeRouteUpdate(async (to, from) => to.params.section !== from.params.section || to.query.view !== from.query.view ? await canDiscard() : true)
watch(() => route.query.connection, (value) => { if (typeof value === 'string' && value !== selectedId.value) void select(value) })
onMounted(async () => {
  try { pendingAuth.value = readMcpPendingAuth(sessionStorage.getItem(MCP_AUTH_STORAGE_KEY)) } catch {}
  if (pendingAuth.value && new URL(location.href).searchParams.has('state')) selectedId.value = pendingAuth.value.connection_id
  window.addEventListener('beforeunload', beforeUnload)
  window.addEventListener('keydown', onKey)
  await refresh()
  if (disposed) return
  await resumeAuth()
  if (disposed) return
  refreshTimer = setInterval(() => { if (!disposed && document.visibilityState === 'visible' && !busy.value && !createOpen.value && !importOpen.value) void refresh() }, 20000)
})
onBeforeUnmount(() => {
  disposed = true; listGeneration++; detailGeneration++; listController?.abort(); detailController?.abort()
  if (refreshTimer) clearInterval(refreshTimer)
  if (reloadTimer) clearTimeout(reloadTimer)
  window.removeEventListener('beforeunload', beforeUnload); window.removeEventListener('keydown', onKey)
  storedToken.value = ''; callbackUrl.value = ''; draft.value = null; createDraft.value = { config: newMcpConfig(), preserveSecrets: [] }
})
defineExpose({ refresh })
</script>

<template>
  <SettingsSectionWorkbench section="mcp-connections" :pages="pages" :active-page="activePage" :default-page="SETTINGS_DEFAULT_SUBPAGE['mcp-connections']">
    <div class="flex min-h-0 min-w-0 flex-col gap-4 lg:h-full">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><span class="rounded-full border border-border/70 px-2.5 py-1">{{ catalog?.connections.length || 0 }} {{ st('Connections') }}</span><span>{{ connectedCount }} {{ st('Connected') }}</span></div>
        <div class="flex flex-wrap items-center gap-2"><IconButton :disabled="loading || busy" :tooltip="st('Refresh connections')" :aria-label="st('Refresh connections')" @click="refresh"><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton><Button variant="outline" size="sm" :disabled="busy" @click="openImport"><RiUpload2Line class="mr-1.5 h-4 w-4" />{{ st('Import connections') }}</Button><Button size="sm" :disabled="busy" @click="openCreate"><RiAddLine class="mr-1.5 h-4 w-4" />{{ st('Add connection') }}</Button></div>
      </div>
      <div v-if="catalog && !catalog.bridge_enabled" class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-500/30 bg-amber-500/5 p-3">
        <div class="min-w-0 flex-1 basis-64"><p class="text-sm font-medium">{{ st('The MCP bridge is disabled') }}</p><p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ st('Enable the bridge to make configured MCP services available. Individual connection settings are preserved.') }}</p></div>
        <div class="flex flex-wrap gap-2"><OptionPicker :model-value="bridgeScope" :options="scopeOptions" :include-empty="false" :disabled="busy" :title="st('Bridge scope')" @update:model-value="bridgeScope = $event === 'global' ? 'global' : 'workspace'" /><Button size="sm" :disabled="busy" @click="setBridge(true)">{{ st('Enable MCP bridge') }}</Button></div>
      </div>
      <p v-if="error" role="alert" class="break-words rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">{{ error }}</p>
      <p v-if="notice" role="status" class="break-words rounded-xl border border-border/70 bg-muted/20 p-3 text-xs text-muted-foreground">{{ notice }}</p>
      <div class="grid min-h-0 min-w-0 flex-1 gap-4 lg:grid-cols-[19rem_minmax(0,1fr)]">
        <aside class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl border border-border/70" :class="showDetail ? 'hidden lg:flex' : ''">
          <div class="space-y-2 border-b border-border/60 p-3"><SearchInput v-model="query" :placeholder="st('Search MCP connections')" /><div class="grid grid-cols-2 gap-2"><OptionPicker v-model="scopeFilter" :options="filterScopes" :include-empty="false" :title="st('Connection scope')" size="sm" /><OptionPicker v-model="statusFilter" :options="statusOptions" :include-empty="false" :title="st('Connection status')" size="sm" /></div></div>
          <div class="min-h-40 flex-1 space-y-1 overflow-y-auto p-1.5 lg:min-h-0">
            <p v-if="loading && !catalog" role="status" class="p-6 text-center text-sm text-muted-foreground">{{ st('Loading connections…') }}</p>
            <div v-else-if="!items.length" class="space-y-2 p-6 text-center"><RiPlugLine class="mx-auto h-7 w-7 text-muted-foreground/60" /><p class="text-sm font-medium">{{ catalog?.connections.length ? st('No matching connections.') : st('Connect your first MCP service') }}</p><p class="text-xs leading-relaxed text-muted-foreground">{{ st('Add a local command or a remote endpoint to bring its tools and resources into Agena.') }}</p></div>
            <button v-for="connection in items" :key="connection.id" type="button" :disabled="busy" :aria-pressed="selectedId === connection.id" class="w-full min-w-0 space-y-2 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60" :class="selectedId === connection.id ? 'border-primary/25 bg-primary/10' : 'border-transparent hover:bg-muted/40'" @click="select(connection.id)">
              <span class="flex items-start justify-between gap-2"><span class="min-w-0 break-words text-sm font-medium">{{ connection.name }}</span><span class="mt-1 h-2 w-2 shrink-0 rounded-full" :class="connection.state === 'connected' ? 'bg-emerald-500' : ['disconnected', 'authentication_required', 'invalid', 'pending_reload'].includes(connection.state) ? 'bg-amber-500' : 'bg-muted-foreground/40'" /></span>
              <span class="block text-xs text-muted-foreground">{{ connection.transport === 'stdio' ? st('Local command') : connection.transport === 'http' ? st('Remote HTTP') : connection.transport }}</span>
              <span class="flex flex-wrap items-center gap-1.5 text-[10px]"><span class="rounded border border-border/70 px-1.5 py-0.5 text-muted-foreground">{{ scopeLabel(connection.scope) }}</span><span class="rounded border px-1.5 py-0.5" :class="stateClass(connection.state)">{{ stateLabel(connection.state) }}</span><span v-if="connection.state === 'connected'" class="text-muted-foreground">{{ connection.tool_count }} {{ st('Tools') }}</span></span>
            </button>
          </div>
          <details v-if="catalog" class="shrink-0 border-t border-border/60 px-3 py-2 text-xs"><summary class="cursor-pointer font-medium text-muted-foreground">{{ st('Server workspace and configuration') }}</summary><div class="mt-2 max-h-52 space-y-3 overflow-y-auto break-all pb-1 text-[11px] text-muted-foreground"><div><span class="font-medium text-foreground">{{ st('Server workspace') }}</span><p class="mt-1 font-mono">{{ catalog.workspace_root }}</p></div><p>{{ st('These connections belong to the running Agena server. Changing the chat directory does not change this workspace.') }}</p><div><span>{{ st('Global configuration') }}</span><p class="mt-1 font-mono">{{ catalog.global_path }}</p></div><div><span>{{ st('Workspace configuration') }}</span><p class="mt-1 font-mono">{{ catalog.workspace_path }}</p></div><div v-if="catalog.bridge_enabled" class="space-y-2 border-t border-border/60 pt-2"><OptionPicker :model-value="bridgeScope" :options="scopeOptions" :include-empty="false" :disabled="busy" :title="st('Bridge scope')" size="sm" @update:model-value="bridgeScope = $event === 'global' ? 'global' : 'workspace'" /><Button variant="outline" size="sm" :disabled="busy" @click="setBridge(false)">{{ st('Disable MCP bridge') }}</Button></div></div></details>
        </aside>
        <section class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl border border-border/70" :class="showDetail ? '' : 'hidden lg:flex'">
          <div class="border-b border-border/60 p-3 lg:hidden"><Button variant="ghost" size="sm" @click="showDetail = false"><RiArrowLeftLine class="mr-1.5 h-4 w-4" />{{ st('Connections') }}</Button></div>
          <div v-if="!detail" class="flex min-h-64 flex-1 items-center justify-center p-8 text-center text-sm text-muted-foreground">{{ detailLoading ? st('Loading connection…') : st('Choose a connection to inspect or configure it.') }}</div>
          <template v-else>
            <header class="space-y-3 border-b border-border/60 p-4"><div class="flex flex-wrap items-start justify-between gap-3"><div class="min-w-0 flex-1"><h2 class="break-words text-base font-semibold">{{ detail.name }}</h2><p class="mt-1 break-all font-mono text-xs text-muted-foreground">{{ detail.secret_paths.includes('/endpoint/url') ? st('Stored endpoint URL') : mcpEndpoint(detail) }}</p><div class="mt-2 flex flex-wrap items-center gap-2 text-[11px]"><span>{{ scopeLabel(detail.scope) }}</span><span class="rounded-full border px-2 py-0.5" :class="stateClass(detail.state)">{{ stateLabel(detail.state) }}</span><span v-if="detail.protocol_version" class="text-muted-foreground">{{ st('Protocol') }} {{ detail.protocol_version }}</span></div></div><div class="flex items-center gap-2"><Button variant="outline" size="sm" :disabled="busy || dirty || !ready" @click="toggleEnabled">{{ detail.enabled ? st('Disable') : st('Enable') }}</Button><IconButton :disabled="busy || !ready" :aria-label="st('Remove connection')" :tooltip="st('Remove connection')" @click="remove"><RiDeleteBinLine class="h-4 w-4 text-destructive" /></IconButton></div></div><div class="flex flex-wrap items-center gap-2"><Button variant="outline" size="sm" :disabled="busy || dirty || !ready || detail.state === 'invalid'" @click="test">{{ action === 'test' ? st('Testing connection…') : st('Test connection') }}</Button><Button variant="outline" size="sm" :disabled="busy || dirty || !ready || !detail.enabled || !detail.effective || !catalog?.bridge_enabled || detail.state === 'pending_reload'" @click="reconnect"><RiRefreshLine class="mr-1.5 h-3.5 w-3.5" :class="action === 'reconnect' ? 'animate-spin' : ''" />{{ st('Reconnect') }}</Button><span v-if="dirty" class="text-xs text-muted-foreground">{{ st('Save changes before testing or reconnecting.') }}</span></div><p v-if="detail.error" class="break-words rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5 text-xs text-amber-800 dark:text-amber-200">{{ detail.error }}</p><p v-if="detail.state === 'pending_reload'" role="status" class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5 text-xs text-amber-800 dark:text-amber-200">{{ st('Saved settings differ from the active runtime. Reload the runtime or check profile overrides.') }}</p><p v-if="detail.state === 'shadowed'" class="text-xs text-muted-foreground">{{ st('A workspace connection with this name takes precedence. This copy remains editable.') }}</p></header>
            <nav class="flex shrink-0 gap-1 overflow-x-auto border-b border-border/60 p-2" :aria-label="st('Connection details')"><button v-for="item in tabs" :key="item.id" type="button" :aria-current="tab === item.id ? 'page' : undefined" class="shrink-0 rounded-lg px-3 py-2 text-xs font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :class="tab === item.id ? 'bg-muted text-foreground shadow-sm' : 'text-muted-foreground hover:bg-muted/40'" @click="tab = item.id">{{ item.label }}</button></nav>
            <div class="min-h-0 flex-1 overflow-y-auto p-4">
              <p v-if="actionError" role="alert" class="mb-4 break-words rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">{{ actionError }}</p>
              <div v-if="testResult" role="status" class="mb-4 space-y-1 rounded-lg border p-3" :class="testResult.ok ? 'border-emerald-500/25 bg-emerald-500/5' : 'border-amber-500/30 bg-amber-500/5'"><p class="text-sm font-medium">{{ testResult.ok ? st('Connection test passed') : st('Connection test failed') }}</p><p class="text-xs text-muted-foreground">{{ st('Elapsed: {value} ms', { value: testResult.elapsed_ms }) }} · {{ testResult.tool_count }} {{ st('Tools') }}<span v-if="testResult.protocol_version"> · {{ testResult.protocol_version }}</span></p><p v-if="testResult.error" class="break-words text-xs">{{ testResult.error }}</p></div>
              <template v-if="tab === 'configuration'">
                <p v-if="changedElsewhere" class="mb-4 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs">{{ st('This connection changed elsewhere. Reload the latest configuration before replacing it.') }}</p>
                <McpConfigEditor v-if="draft" :key="selectedId + ':' + editorEpoch" v-model="draft" :disabled="busy || !ready" @validity="editorValid = $event" />
                <ul v-if="dirty && draftProblems.length" class="mt-4 list-inside list-disc space-y-1 text-xs text-destructive"><li v-for="problem in draftProblems" :key="problem">{{ st(problem) }}</li></ul>
                <section v-if="detail.transport === 'http' && ['o_auth', 'bearer_from_store'].includes(authKind)" class="mt-5 space-y-3 rounded-xl border border-border/70 bg-muted/10 p-4"><h3 class="text-sm font-semibold">{{ st('Credentials') }}</h3><p v-if="dirty" class="text-xs text-muted-foreground">{{ st('Save the connection configuration before changing credentials.') }}</p>
                  <template v-if="authKind === 'o_auth'"><div class="flex flex-wrap gap-2"><Button size="sm" :disabled="busy || dirty || !ready" @click="startAuth()">{{ action === 'auth-start' ? st('Starting sign-in…') : st('Sign in') }}</Button><Button size="sm" variant="outline" :disabled="busy || dirty || !ready" @click="startAuth(true)">{{ st('Use manual authorization') }}</Button><Button size="sm" variant="ghost" :disabled="busy || dirty || !ready" @click="logout()">{{ st('Sign out') }}</Button></div>
                    <div v-if="pendingAuth?.connection_id === detail.id" class="space-y-3 border-t border-border/60 pt-3"><p class="text-xs leading-relaxed text-muted-foreground">{{ st('Open the authorization link, finish signing in, then paste the complete callback URL here.') }}</p><a v-if="authorizationUrl" :href="authorizationUrl" target="_blank" rel="noopener noreferrer" class="inline-flex items-center gap-1.5 text-xs font-medium text-primary underline underline-offset-4">{{ st('Open authorization link') }}<RiExternalLinkLine class="h-3.5 w-3.5" /></a><label class="block space-y-1.5" :for="fieldId + '-callback'"><span class="text-xs font-medium">{{ st('Callback URL') }}</span><input :id="fieldId + '-callback'" v-model="callbackUrl" type="url" :disabled="busy" autocomplete="off" spellcheck="false" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" /></label><div class="flex flex-wrap gap-2"><Button size="sm" :disabled="busy || !callbackUrl" @click="completeAuth()">{{ st('Complete sign-in') }}</Button><Button size="sm" variant="ghost" :disabled="busy" @click="logout(true)">{{ st('Cancel sign-in') }}</Button></div></div>
                  </template>
                  <template v-else><label class="block space-y-1.5" :for="fieldId + '-stored-token'"><span class="text-xs font-medium">{{ st('New stored bearer token') }}</span><input :id="fieldId + '-stored-token'" v-model="storedToken" type="password" :disabled="busy || dirty" autocomplete="new-password" class="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs" /></label><div class="flex flex-wrap gap-2"><Button size="sm" :disabled="busy || dirty || !storedToken" @click="credential()">{{ st('Save token') }}</Button><Button size="sm" variant="outline" :disabled="busy || dirty" @click="credential(true)">{{ st('Remove token') }}</Button></div></template>
                  <details v-if="detail.oauth_health"><summary class="cursor-pointer text-xs text-muted-foreground">{{ st('Authentication status') }}</summary><pre class="mt-2 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px]">{{ JSON.stringify(detail.oauth_health, null, 2) }}</pre></details>
                </section>
                <details v-if="detail.server_info || detail.capabilities" class="mt-5 rounded-xl border border-border/70"><summary class="cursor-pointer p-3 text-xs font-medium">{{ st('Server information') }}</summary><pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words border-t border-border/60 p-3 font-mono text-[11px] leading-relaxed">{{ JSON.stringify({ server: detail.server_info, capabilities: detail.capabilities }, null, 2) }}</pre></details>
              </template>
              <McpCapabilitiesPanel v-else :key="selectedId + ':' + tab" :connection-id="selectedId" :enabled="detail.state === 'connected'" :kind="tab === 'resources' ? 'resources' : tab === 'prompts' ? 'prompts' : 'tools'" />
            </div>
            <div v-if="draft && tab === 'configuration'" class="shrink-0 border-t border-border/60 p-3"><SettingsSaveBar :dirty="dirty" :saving="action === 'save'" :disabled="busy || !ready" :save-disabled="!editorValid || Boolean(draftProblems.length) || changedElsewhere" @save="save" @discard="discard" /></div>
          </template>
        </section>
      </div>
    </div>
    <Dialog :open="createOpen" :title="st('Add MCP connection')" :description="st('Configure a local process or a remote service for this Agena server.')" max-width="max-w-2xl" @update:open="(value) => { if (!value) void closeCreate() }">
      <div class="space-y-5"><div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_10rem]"><label class="space-y-1.5" :for="fieldId + '-name'"><span class="text-xs font-medium">{{ st('Connection name') }}</span><input :id="fieldId + '-name'" v-model="createName" :disabled="busy" autofocus autocomplete="off" spellcheck="false" class="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm" /></label><div class="space-y-1.5"><span class="text-xs font-medium">{{ st('Connection scope') }}</span><OptionPicker :model-value="createScope" :options="scopeOptions" :include-empty="false" :disabled="busy" :title="st('Connection scope')" @update:model-value="createScope = $event === 'global' ? 'global' : 'workspace'" /></div></div><p v-if="createScope === 'workspace' && catalog" class="break-all text-xs text-muted-foreground">{{ st('Server workspace') }}: {{ catalog.workspace_root }}</p><McpConfigEditor v-if="createOpen" v-model="createDraft" :disabled="busy" @validity="createValid = $event" /><p v-if="createError" role="alert" class="break-words text-xs text-destructive">{{ createError }}</p><ul v-if="createName.trim() && createProblems.length" class="list-inside list-disc space-y-1 text-xs text-muted-foreground"><li v-for="problem in createProblems" :key="problem">{{ st(problem) }}</li></ul></div>
      <template #footer><Button variant="ghost" :disabled="busy" @click="closeCreate">{{ st('Cancel') }}</Button><div class="flex-1" /><Button :disabled="busy || !createName.trim() || !createValid || Boolean(createProblems.length)" @click="create">{{ action === 'create' ? st('Adding connection…') : st('Add connection') }}</Button></template>
    </Dialog>
    <McpImportDialog :open="importOpen" :workspace-root="catalog?.workspace_root || ''" @update:open="importOpen = $event" @dirty="importDirty = $event" @installed="imported" />
  </SettingsSectionWorkbench>
</template>
