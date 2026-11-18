<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, watch } from 'vue'
import { RiAddLine, RiArrowDownSLine, RiDeleteBinLine, RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import PermissionSettingsCard from './permissions/PermissionSettingsCard.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'
import { useChatStore } from '@/stores/chat'
import { getSession } from '@/stores/chat/api'
import { lastSessionId } from '@/lib/lastSession'
import { useDirectoryStore } from '@/stores/directory'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { normalizeDirForCompare } from '@/features/sessions/model/labels'
import { permissionRuleMatchesScope } from '@/lib/permissionRuleScope'

type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
type PermissionScope = 'effective' | 'global' | 'workspace' | 'session'

type PermissionRule = {
  id: number
  action_key: string
  subject_kind: string
  tool_name?: string | null
  qualifier?: string | null
  path_access_kind?: string | null
  target_path?: string | null
  network_target?: string | null
  workspace_id?: number | null
  session_id?: number | null
  mode: PermissionMode
  scope: string
  source: string
  reason?: string | null
  revoked_at?: string | null
  created_at: string
  updated_at: string
}

type PermissionRulePage = {
  items?: PermissionRule[]
  page?: {
    limit: number
    returned: number
    has_more: boolean
    next_cursor?: string | null
  }
}

const PERMISSION_RULE_BATCH_SIZE = 200

const props = defineProps<{ scope: PermissionScope; sessionId?: string; compact?: boolean }>()
const toasts = useToastsStore()
const chat = useChatStore()
const directory = useDirectoryStore()
const directorySessions = useDirectorySessionStore()

const rules = ref<PermissionRule[]>([])
const hasMore = ref(false)
const selectedIds = ref<Set<number>>(new Set())

const createBusy = ref(false)
const createError = ref('')
const createExpanded = ref(false)
const createForm = ref<HTMLFormElement | null>(null)
const newToolName = ref('')
const newQualifier = ref('')
const newMode = ref<PermissionMode>('ask')
const bulkMode = ref<PermissionMode>('ask')
const bulkBusy = ref(false)
const rowBusy = ref<number | null>(null)
const busy = computed(() => createBusy.value || bulkBusy.value || rowBusy.value !== null)
const query = ref('')
const stateFilter = ref('all')
const stateOptions = computed(() => [
  { value: 'all', label: st('All rules') },
  { value: 'active', label: st('Active') },
  { value: 'revoked', label: st('revoked') },
])
const activeSessionId = computed(() => {
  // A settings page names the conversation it edits; the chat selection is only
  // the fallback when this panel is embedded in a conversation surface.
  const id = Number(String(props.sessionId || '').trim() || lastSessionId.value || chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
function workspaceId(value: unknown): number | null {
  const id = Number(value)
  return Number.isSafeInteger(id) && id > 0 ? id : null
}
const targetWorkspace = ref<{ sessionId: number; workspaceId: number | null } | null>(null)
const targetWorkspaceError = ref('')
let targetWorkspaceGeneration = 0
let targetWorkspaceController: AbortController | undefined
watch(
  () => [props.scope, activeSessionId.value] as const,
  async ([scope, sessionId]) => {
    const generation = ++targetWorkspaceGeneration
    targetWorkspaceController?.abort()
    targetWorkspace.value = null
    targetWorkspaceError.value = ''
    if ((scope !== 'session' && scope !== 'effective') || !sessionId) return
    targetWorkspace.value = {
      sessionId,
      workspaceId: workspaceId(chat.getSessionById(String(sessionId))?.workspace_id),
    }
    const controller = new AbortController()
    targetWorkspaceController = controller
    try {
      const session = await getSession(String(sessionId), controller.signal)
      if (
        generation !== targetWorkspaceGeneration ||
        controller.signal.aborted ||
        activeSessionId.value !== sessionId ||
        props.scope !== scope
      )
        return
      chat.cacheSessionMetadata([session])
      const current = chat.getSessionById(String(sessionId))
      targetWorkspace.value = {
        sessionId,
        workspaceId: workspaceId(
          current && Object.hasOwn(current, 'workspace_id') ? current.workspace_id : session.workspace_id,
        ),
      }
    } catch (reason) {
      if (generation !== targetWorkspaceGeneration || controller.signal.aborted) return
      targetWorkspace.value = { sessionId, workspaceId: null }
      targetWorkspaceError.value = reason instanceof Error ? reason.message : String(reason)
    }
  },
  { immediate: true },
)
onScopeDispose(() => {
  targetWorkspaceGeneration += 1
  targetWorkspaceController?.abort()
})
const activeWorkspaceId = computed(() => {
  if (props.scope === 'session' || props.scope === 'effective') {
    return targetWorkspace.value?.sessionId === activeSessionId.value ? targetWorkspace.value.workspaceId : null
  }
  const sessionWorkspaceId = Number(chat.selectedSession?.workspace_id)
  if (Number.isSafeInteger(sessionWorkspaceId) && sessionWorkspaceId > 0) return sessionWorkspaceId
  if (typeof window === 'undefined' || !window.location) return null
  const currentPath = normalizeDirForCompare(directory.currentDirectory || '')
  if (!currentPath) return null
  const workspace = directorySessions.visibleDirectories.find(
    (item) => normalizeDirForCompare(item.path) === currentPath,
  )
  const workspaceId = Number(workspace?.id)
  return Number.isSafeInteger(workspaceId) && workspaceId > 0 ? workspaceId : null
})
const canEdit = computed(
  () => props.scope === 'global' || (props.scope === 'workspace' && activeWorkspaceId.value !== null),
)
function scopeLabel(scope: string) {
  if (scope === 'global') return st('Global')
  if (scope === 'workspace') return st('Workspace')
  if (scope === 'session') return st('Current Session Permission')
  return scope
}

const modeOptions = computed(() => [
  { value: 'allow', label: st('Allow'), description: st('Approve matching tool calls.') },
  { value: 'auto', label: st('Auto'), description: st('Let Agena evaluate matching calls automatically.') },
  { value: 'ask', label: st('Ask'), description: st('Request confirmation before running.') },
  { value: 'deny', label: st('Deny'), description: st('Block matching tool calls.') },
])
const networkModeOptions = computed(() =>
  modeOptions.value.filter((option) => ['allow', 'deny'].includes(option.value)),
)

const canCreate = computed(() => canEdit.value && !createBusy.value && newToolName.value.trim().length > 0)

const sortedRules = computed(() =>
  [...rules.value]
    .sort((a, b) => {
      const revokedOrder = Number(Boolean(a.revoked_at)) - Number(Boolean(b.revoked_at))
      return revokedOrder || b.id - a.id
    })
    .filter((rule) => permissionRuleMatchesScope(rule, props.scope, activeWorkspaceId.value, activeSessionId.value)),
)
const filteredRules = computed(() =>
  sortedRules.value.filter((rule) => {
    if (stateFilter.value === 'active' && rule.revoked_at) return false
    if (stateFilter.value === 'revoked' && !rule.revoked_at) return false
    return `${ruleTitle(rule)} ${rule.reason || ''} ${rule.source} ${rule.mode}`
      .toLowerCase()
      .includes(query.value.trim().toLowerCase())
  }),
)
const selectableRules = computed(() => filteredRules.value.filter((rule) => canRemoveRule(rule)))
const selectedRules = computed(() => selectableRules.value.filter((rule) => selectedIds.value.has(rule.id)))
const selectedActiveRules = computed(() => selectedRules.value.filter((rule) => !rule.revoked_at))
const bulkModeOptions = computed(() =>
  selectedActiveRules.value.some((rule) => rule.subject_kind === 'network_access')
    ? networkModeOptions.value
    : modeOptions.value,
)
watch(bulkModeOptions, (options) => {
  if (!options.some((option) => option.value === bulkMode.value)) bulkMode.value = 'deny'
})
const allSelected = computed(
  () => selectableRules.value.length > 0 && selectableRules.value.every((rule) => selectedIds.value.has(rule.id)),
)

watch(selectableRules, (available) => {
  const allowed = new Set(available.map((rule) => rule.id))
  selectedIds.value = new Set([...selectedIds.value].filter((id) => allowed.has(id)))
})

function canRemoveRule(rule: PermissionRule): boolean {
  return canEdit.value && permissionRuleMatchesScope(rule, props.scope, activeWorkspaceId.value, activeSessionId.value)
}

function ruleTitle(rule: PermissionRule): string {
  if (rule.subject_kind === 'tool' && rule.tool_name) {
    return rule.qualifier
      ? st('{tool_name} · {qualifier}', { tool_name: rule.tool_name, qualifier: rule.qualifier })
      : rule.tool_name
  }
  if (rule.subject_kind === 'path_access') {
    return st('{path} · {target_path}', { path: rule.path_access_kind || 'path', target_path: rule.target_path || '' })
  }
  if (rule.subject_kind === 'network_access') return rule.network_target || 'Network access'
  return rule.action_key
}

const { loading, error, refresh } = usePaneRead(
  (signal) => apiJson<PermissionRulePage>('/api/v1/permission-rules?limit=200', { signal }),
  (data) => {
    rules.value = Array.isArray(data?.items) ? data.items : []
    hasMore.value = data?.page?.has_more === true
  },
)
defineExpose({ refresh, busy })
watch(
  () => [props.scope, activeWorkspaceId.value, activeSessionId.value],
  () => {
    clearSelection()
    createError.value = ''
    createExpanded.value = false
  },
)

async function setRuleMode(rule: PermissionRule, mode: string) {
  if (!canRemoveRule(rule) || rule.revoked_at || rowBusy.value !== null || bulkBusy.value || mode === rule.mode) return
  if (rule.subject_kind === 'network_access' && !networkModeOptions.value.some((option) => option.value === mode))
    return
  rowBusy.value = rule.id
  try {
    await apiJson('/api/v1/permission-rules/batch', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ rule_ids: [rule.id], scope: props.scope, operation: { action: 'set_mode', mode } }),
    })
    await refresh()
  } catch (reason) {
    toasts.push('error', reason instanceof Error ? reason.message : String(reason))
  } finally {
    rowBusy.value = null
  }
}

function toggleRuleSelection(id: number, checked: boolean) {
  const next = new Set(selectedIds.value)
  if (checked) next.add(id)
  else next.delete(id)
  selectedIds.value = next
}

function toggleAllSelection() {
  if (allSelected.value) {
    selectedIds.value = new Set(
      [...selectedIds.value].filter((id) => !selectableRules.value.some((rule) => rule.id === id)),
    )
  } else {
    selectedIds.value = new Set([...selectedIds.value, ...selectableRules.value.map((rule) => rule.id)])
  }
}

function clearSelection() {
  selectedIds.value = new Set()
}

function clearFilters() {
  query.value = ''
  stateFilter.value = 'all'
}

async function toggleCreateForm() {
  if (!canEdit.value || createBusy.value) return
  createExpanded.value = !createExpanded.value
  if (!createExpanded.value) return
  await nextTick()
  createForm.value?.querySelector<HTMLInputElement>('input:not(:disabled)')?.focus()
}

async function applyBulkOperation(action: 'delete' | 'set_mode') {
  const targetRules = action === 'set_mode' ? selectedActiveRules.value : selectedRules.value
  if (!targetRules.length || bulkBusy.value || rowBusy.value !== null) return
  if (action === 'set_mode' && !bulkModeOptions.value.some((option) => option.value === bulkMode.value)) return
  const ruleIds = targetRules.map((rule) => rule.id)
  const scope = props.scope
  const sessionId = activeSessionId.value
  const operation = action === 'delete' ? { action: 'delete' } : { action: 'set_mode', mode: bulkMode.value }
  bulkBusy.value = true
  try {
    let affectedCount = 0
    let missingCount = 0
    for (let offset = 0; offset < ruleIds.length; offset += PERMISSION_RULE_BATCH_SIZE) {
      const response = await apiJson<{ affected?: PermissionRule[]; missing_ids?: number[] }>(
        '/api/v1/permission-rules/batch',
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            rule_ids: ruleIds.slice(offset, offset + PERMISSION_RULE_BATCH_SIZE),
            scope,
            ...(scope === 'session' && sessionId ? { session_id: sessionId } : {}),
            operation,
          }),
        },
      )
      affectedCount += Array.isArray(response?.affected) ? response.affected.length : 0
      missingCount += Array.isArray(response?.missing_ids) ? response.missing_ids.length : 0
    }
    clearSelection()
    if (missingCount) {
      toasts.push('info', st('Some selected rules had changed and were skipped.'))
    } else {
      toasts.push(
        'success',
        action === 'delete'
          ? st('{count} permission rules removed', { count: affectedCount })
          : st('{count} permission rules updated', { count: affectedCount }),
      )
    }
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
    await refresh().catch(() => {})
  } finally {
    bulkBusy.value = false
  }
}

async function createRule() {
  if (!canCreate.value) return
  if (props.scope !== 'global' && props.scope !== 'workspace') return
  if (props.scope === 'workspace' && activeWorkspaceId.value === null) return
  createBusy.value = true
  createError.value = ''
  try {
    const qualifier = newQualifier.value.trim()
    await apiJson('/api/v1/permission-rules', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        subject_kind: 'tool',
        tool_name: newToolName.value.trim(),
        ...(qualifier ? { qualifier } : {}),
        mode: newMode.value,
        scope: props.scope,
      }),
    })
    newToolName.value = ''
    newQualifier.value = ''
    createExpanded.value = false
    toasts.push('success', st('Permission rule created'))
    await refresh()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    createError.value = message
    toasts.push('error', message)
  } finally {
    createBusy.value = false
  }
}

async function removeRule(id: number) {
  const rule = rules.value.find((candidate) => candidate.id === id)
  if (!rule || !canRemoveRule(rule) || rowBusy.value !== null || bulkBusy.value) return
  rowBusy.value = id
  try {
    await apiJson(`/api/v1/permission-rules/${id}`, { method: 'DELETE' })
    toasts.push('success', st('Permission rule removed'))
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    rowBusy.value = null
  }
}
</script>

<template>
  <component
    :is="compact ? 'section' : PermissionSettingsCard"
    v-bind="
      compact
        ? {}
        : {
            title: $st('Saved approval rules'),
            description: $st('Rules remembered from approvals. Changes here take effect immediately.'),
            count: sortedRules.length,
            icon: RiShieldCheckLine,
          }
    "
    class="saved-rules"
    :class="{ 'saved-rules--compact': compact }"
    data-saved-rules
  >
    <template v-if="!compact" #actions>
      <IconButton
        variant="ghost"
        size="sm"
        :tooltip="$st('Refresh permission rules')"
        :aria-label="$st('Refresh permission rules')"
        :disabled="loading"
        @click="refresh"
        ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''"
      /></IconButton>
    </template>
    <div v-if="compact" class="saved-rules-heading">
      <div class="flex min-w-0 items-center gap-1.5">
        <RiShieldCheckLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <h3 class="min-w-0 text-xs font-semibold">{{ $st('Saved approval rules') }}</h3>
        <span class="text-[11px] tabular-nums text-muted-foreground">{{ sortedRules.length }}</span>
      </div>
      <div class="flex items-center gap-0.5">
        <IconButton
          v-if="canEdit"
          variant="ghost"
          size="sm"
          :tooltip="$st('Create tool rule')"
          :aria-label="$st('Create tool rule')"
          :aria-expanded="createExpanded"
          :disabled="createBusy"
          @click="toggleCreateForm"
          ><RiAddLine class="h-4 w-4" />
        </IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          :tooltip="$st('Refresh permission rules')"
          :aria-label="$st('Refresh permission rules')"
          :disabled="loading"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
      <span
        class="saved-rules-immediate text-[11px] text-muted-foreground"
        :title="$st('Rules remembered from approvals. Changes here take effect immediately.')"
        >{{ $st('Applied immediately') }}</span
      >
    </div>
    <p
      v-if="scope === 'workspace' && !activeWorkspaceId"
      :class="compact ? 'saved-rules-empty' : 'mb-3 text-xs text-muted-foreground'"
    >
      {{ $st('Select a workspace to view and manage its permission rules.') }}
    </p>
    <p
      v-if="!compact && sortedRules.some((rule) => rule.subject_kind === 'network_access')"
      class="mb-3 text-xs text-muted-foreground"
    >
      {{
        $st(
          'Network decisions accept allow or deny and literal IP targets such as 192.0.2.10:443. Use tcp://, udp:// or icmp:// for a protocol; bare IP covers configured protocols and ports. Configure CIDRs in network settings.',
        )
      }}
    </p>
    <div v-if="sortedRules.length" class="saved-rules-controls" data-saved-rules-controls>
      <SearchInput
        v-model="query"
        class="saved-rules-search min-w-0"
        :placeholder="$st('Search rules…')"
        :input-aria-label="$st('Search saved approval rules')"
        :show-search-button="false"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
      />
      <div class="saved-rules-status min-w-0">
        <OptionPicker
          v-model="stateFilter"
          :options="stateOptions"
          :include-empty="false"
          :title="$st('Rule status')"
        />
      </div>
      <label
        v-if="selectableRules.length"
        class="saved-rules-select-all flex items-center gap-2 text-xs text-muted-foreground"
        :title="$st('Select all')"
        ><input
          type="checkbox"
          class="accent-primary"
          :checked="allSelected"
          :indeterminate="selectedRules.length > 0 && !allSelected"
          :disabled="bulkBusy || rowBusy !== null"
          :aria-label="$st('Select all permission rules')"
          @change="toggleAllSelection"
        /><span :class="compact ? 'sr-only' : ''">{{ $st('Select all') }}</span></label
      >
    </div>
    <div
      v-if="selectedRules.length"
      class="saved-rules-bulk mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-primary/20 bg-primary/5 p-2.5"
    >
      <span class="text-xs font-medium">{{ $st('{count} selected', { count: selectedRules.length }) }}</span>
      <Button variant="ghost" size="sm" :disabled="bulkBusy" @click="clearSelection">{{ $st('Select none') }}</Button>
      <div class="flex flex-1 flex-wrap items-center justify-end gap-2">
        <div class="w-32">
          <OptionPicker
            v-model="bulkMode"
            :options="bulkModeOptions"
            :include-empty="false"
            :title="$st('Set mode for selected rules')"
            :disabled="!selectedActiveRules.length || bulkBusy || rowBusy !== null"
            size="sm"
          />
        </div>
        <Button
          variant="outline"
          size="sm"
          :disabled="!selectedActiveRules.length || bulkBusy || rowBusy !== null"
          @click="applyBulkOperation('set_mode')"
          >{{ $st('Apply') }}</Button
        >
        <ConfirmPopover
          :title="$st('Remove selected permission rules?')"
          :description="$st('Remove {count} selected permission rules?', { count: selectedRules.length })"
          :confirm-text="$st('Remove selected')"
          :cancel-text="$st('Cancel')"
          variant="destructive"
          :confirm-disabled="bulkBusy || rowBusy !== null"
          @confirm="applyBulkOperation('delete')"
        >
          <Button variant="ghost" size="sm" class="text-destructive" :disabled="bulkBusy || rowBusy !== null">{{
            $st('Remove selected')
          }}</Button>
        </ConfirmPopover>
      </div>
    </div>
    <div
      v-if="error || targetWorkspaceError"
      role="alert"
      class="mb-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {{ error || targetWorkspaceError }}
    </div>
    <p
      v-if="loading && !rules.length"
      role="status"
      :class="compact ? 'saved-rules-empty' : 'py-3 text-sm text-muted-foreground'"
    >
      {{ $st('Loading permission rules...') }}
    </p>
    <p
      v-else-if="!sortedRules.length && !error && !(scope === 'workspace' && !activeWorkspaceId)"
      :class="compact ? 'saved-rules-empty' : 'py-3 text-sm text-muted-foreground'"
    >
      {{ $st('No permission rules configured.') }}
    </p>
    <div
      v-else-if="sortedRules.length && !filteredRules.length && !error"
      :class="
        compact
          ? 'saved-rules-empty flex flex-wrap items-center justify-between gap-1'
          : 'rounded-lg bg-muted/20 px-4 py-5 text-center'
      "
    >
      <p class="text-sm text-muted-foreground">{{ $st('No matching rules.') }}</p>
      <Button variant="ghost" size="sm" class="mt-1" @click="clearFilters">{{ $st('Clear filters') }}</Button>
    </div>
    <div v-if="filteredRules.length" class="saved-rules-list grid gap-2" role="list">
      <div
        v-for="rule in filteredRules"
        :key="rule.id"
        class="saved-rule-row"
        :class="[
          compact ? 'saved-rule-row--compact' : 'rounded-lg border border-border/50 bg-background/20 p-3',
          rule.revoked_at ? 'opacity-60' : '',
        ]"
        :data-saved-rule-id="rule.id"
        role="listitem"
      >
        <input
          v-if="canRemoveRule(rule)"
          type="checkbox"
          class="rule-checkbox accent-primary"
          :checked="selectedIds.has(rule.id)"
          :disabled="bulkBusy || rowBusy !== null"
          :aria-label="$st('Select permission rule {id}', { id: rule.id })"
          @change="toggleRuleSelection(rule.id, ($event.target as HTMLInputElement).checked)"
        />
        <div class="saved-rule-heading min-w-0">
          <div
            class="saved-rule-target break-all font-mono text-xs font-medium"
            :class="compact ? 'line-clamp-2' : ''"
            :title="ruleTitle(rule)"
          >
            {{ ruleTitle(rule) }}
          </div>
          <p
            v-if="rule.reason"
            class="saved-rule-reason mt-1 line-clamp-2 text-xs text-muted-foreground"
            :title="rule.reason"
          >
            {{ rule.reason }}
          </p>
          <div
            class="saved-rule-metadata mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground"
            :title="`${scopeLabel(rule.scope)} · #${rule.id} · ${rule.source || ''}`"
          >
            <span>{{ scopeLabel(rule.scope) }}</span
            ><span aria-hidden="true">·</span><span>#{{ rule.id }}</span
            ><span v-if="rule.source" :title="$st('Source')">{{ rule.source }}</span
            ><span v-if="!compact && rule.subject_kind === 'network_access'">{{ $st('Mode') }}: {{ rule.mode }}</span
            ><span v-if="rule.revoked_at" class="text-destructive">{{ $st('revoked') }}</span>
          </div>
        </div>
        <div class="saved-rule-mode">
          <OptionPicker
            v-if="canRemoveRule(rule) && !rule.revoked_at"
            :model-value="rule.mode"
            :options="rule.subject_kind === 'network_access' ? networkModeOptions : modeOptions"
            :include-empty="false"
            :title="$st('Permission mode')"
            :disabled="bulkBusy || rowBusy !== null"
            size="sm"
            @update:model-value="setRuleMode(rule, $event)"
          />
          <span
            v-else
            class="inline-flex rounded-md border border-border/60 px-2 py-1 text-xs"
            :class="rule.mode === 'deny' ? 'bg-destructive/5 text-destructive' : 'bg-muted/20 text-muted-foreground'"
            >{{ modeOptions.find((option) => option.value === rule.mode)?.label || rule.mode }}</span
          >
        </div>
        <div v-if="canRemoveRule(rule)" class="saved-rule-remove">
          <ConfirmPopover
            :title="$st('Remove permission rule?')"
            :description="ruleTitle(rule)"
            :confirm-text="$st('Remove rule')"
            :cancel-text="$st('Cancel')"
            variant="destructive"
            :confirm-disabled="bulkBusy || rowBusy !== null"
            @confirm="removeRule(rule.id)"
          >
            <IconButton
              class="text-muted-foreground hover:text-destructive"
              variant="ghost"
              size="sm"
              :tooltip="$st('Remove rule')"
              :aria-label="$st('Remove rule')"
              :disabled="bulkBusy || rowBusy !== null"
              ><RiDeleteBinLine class="h-4 w-4"
            /></IconButton>
          </ConfirmPopover>
        </div>
      </div>
    </div>
    <p v-if="hasMore" class="mt-3 text-xs text-muted-foreground">
      {{ $st('More than 200 rules exist. Refine or manage older rules through the CLI.') }}
    </p>
    <details
      v-if="compact && sortedRules.some((rule) => rule.subject_kind === 'network_access')"
      class="saved-rules-help group text-[11px] text-muted-foreground"
    >
      <summary
        class="flex cursor-pointer list-none items-center gap-1 py-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {{ $st('Network access') }}
        <RiArrowDownSLine class="h-3 w-3 transition-transform group-open:rotate-180" />
      </summary>
      <p class="pb-2 leading-relaxed">
        {{
          $st(
            'Network decisions accept allow or deny and literal IP targets such as 192.0.2.10:443. Use tcp://, udp:// or icmp:// for a protocol; bare IP covers configured protocols and ports. Configure CIDRs in network settings.',
          )
        }}
      </p>
    </details>
    <component
      :is="compact ? 'div' : 'details'"
      v-if="canEdit && (!compact || createExpanded)"
      class="saved-rule-create group mt-3 border-t border-border/50 pt-3"
    >
      <summary
        v-if="!compact"
        class="flex cursor-pointer list-none items-center gap-2 rounded-md py-1 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <RiAddLine class="h-4 w-4" />{{ $st('Create tool rule')
        }}<RiArrowDownSLine class="ml-auto h-4 w-4 transition-transform group-open:rotate-180" />
      </summary>
      <form ref="createForm" class="saved-rule-form" :class="compact ? '' : 'mt-3'" @submit.prevent="createRule">
        <label class="grid min-w-0 gap-1.5"
          ><span class="text-xs text-muted-foreground">{{ $st('Tool name') }}</span
          ><Input v-model="newToolName" placeholder="agena.shell.exec" :disabled="createBusy" class="font-mono text-xs"
        /></label>
        <label class="grid min-w-0 gap-1.5"
          ><span class="text-xs text-muted-foreground">{{ $st('Qualifier') }}</span
          ><Input
            v-model="newQualifier"
            :placeholder="$st('Optional command or operation')"
            :disabled="createBusy"
            class="font-mono text-xs"
        /></label>
        <label class="grid min-w-0 gap-1.5"
          ><span class="text-xs text-muted-foreground">{{ $st('Mode') }}</span
          ><OptionPicker
            v-model="newMode"
            :options="modeOptions"
            :title="$st('Permission mode')"
            :include-empty="false"
            :disabled="createBusy"
        /></label>
        <div class="saved-rule-create-actions flex items-center justify-end gap-1">
          <Button
            v-if="compact"
            type="button"
            variant="ghost"
            size="sm"
            :disabled="createBusy"
            @click="toggleCreateForm"
            >{{ $st('Cancel') }}</Button
          >
          <Button type="submit" variant="outline" size="sm" class="h-9" :disabled="!canCreate">{{
            createBusy ? $st('Creating…') : $st('Create rule')
          }}</Button>
        </div>
      </form>
      <p v-if="createError" class="mt-2 break-words text-xs text-destructive" role="alert">{{ createError }}</p>
    </component>
  </component>
</template>

<style scoped>
@reference '../../style.css';
.saved-rules {
  min-width: 0;
  container: saved-rules / inline-size;
}
.saved-rules-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  margin-block-end: 0.75rem;
}
.saved-rules-search {
  flex: 1 1 12rem;
}
.saved-rules-status {
  width: 9rem;
}
.saved-rules-heading {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 0 0.375rem;
  padding-block: 0.375rem;
}
.saved-rules-immediate {
  grid-column: 1 / -1;
  padding-inline-start: 1.375rem;
}
.saved-rules-empty {
  @apply text-xs text-muted-foreground;
  padding-block: 0.375rem;
}
.saved-rule-row {
  display: grid;
  grid-template-columns: 1.25rem minmax(0, 1fr) 2rem;
  align-items: center;
  gap: 0.75rem;
}
.rule-checkbox {
  grid-column: 1;
  grid-row: 1;
}
.saved-rule-heading {
  grid-column: 2;
  grid-row: 1;
}
.saved-rule-mode {
  grid-column: 2;
  width: 10rem;
  max-width: 100%;
}
.saved-rule-remove {
  grid-column: 3;
  grid-row: 1;
}
.saved-rule-form {
  display: grid;
  gap: 0.75rem;
  min-width: 0;
  align-items: end;
}
summary::-webkit-details-marker {
  display: none;
}
@container permissions (min-width: 40rem) {
  .saved-rule-row {
    grid-template-columns: 1.25rem minmax(0, 1fr) 9rem 2rem;
  }
  .saved-rule-mode {
    grid-column: 3;
    grid-row: 1;
    width: auto;
  }
  .saved-rule-remove {
    grid-column: 4;
  }
  .saved-rule-form {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) 9rem auto;
  }
}
.saved-rules--compact .saved-rules-controls {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 6.75rem auto;
  gap: 0.375rem;
  margin-block-end: 0.25rem;
}
.saved-rules--compact .saved-rules-status {
  width: auto;
}
.saved-rules--compact .saved-rules-select-all {
  justify-content: center;
}
.saved-rules--compact .saved-rules-bulk {
  gap: 0.375rem;
  padding: 0.375rem;
  margin-block-end: 0.25rem;
}
.saved-rules--compact .saved-rules-list {
  gap: 0;
}
.saved-rule-row--compact {
  grid-template-columns: 1rem minmax(0, 1fr) 6.75rem var(--saved-rule-action-width, 2rem);
  gap: 0.25rem 0.375rem;
  padding-block: 0.375rem;
}
.saved-rule-row--compact + .saved-rule-row--compact {
  @apply border-t border-border/50;
}
.saved-rule-row--compact .saved-rule-heading {
  display: contents;
}
.saved-rule-row--compact .saved-rule-target {
  grid-column: 2 / 4;
  grid-row: 1;
  min-width: 0;
}
.saved-rule-row--compact .saved-rule-metadata {
  grid-column: 2;
  grid-row: 2;
  min-width: 0;
  margin: 0;
  flex-wrap: nowrap;
  gap: 0.25rem;
  overflow: hidden;
  white-space: nowrap;
}
.saved-rule-row--compact .saved-rule-metadata > span {
  flex-shrink: 0;
}
.saved-rule-row--compact .saved-rule-metadata > span:last-child {
  min-width: 0;
  flex-shrink: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}
.saved-rule-row--compact .saved-rule-mode {
  grid-column: 3;
  grid-row: 2;
  width: auto;
  min-width: 0;
}
.saved-rule-row--compact .saved-rule-remove {
  grid-column: 4;
  grid-row: 1 / 3;
}
.saved-rule-row--compact .rule-checkbox {
  grid-column: 1;
  grid-row: 1 / 3;
}
.saved-rule-row--compact .saved-rule-reason {
  grid-column: 2 / 4;
  grid-row: 3;
  margin: 0;
  font-size: 0.8125rem;
}
.saved-rules--compact :deep(.text-xs),
.saved-rules--compact :deep([class~='text-[11px]']),
.saved-rules--compact .saved-rules-empty {
  font-size: 0.8125rem;
}
.saved-rules--compact .saved-rule-create {
  margin-block-start: 0.375rem;
  padding-block-start: 0.5rem;
}
.saved-rules--compact .saved-rule-form {
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 0.5rem;
}
.saved-rules--compact .saved-rule-form > label {
  gap: 0.25rem;
}
.saved-rules--compact .saved-rule-form > label:nth-child(-n + 2) {
  grid-column: 1 / -1;
}
@container saved-rules (min-width: 40rem) {
  .saved-rule-row--compact {
    grid-template-columns: 1rem minmax(0, 1fr) 8rem 6.75rem var(--saved-rule-action-width, 2rem);
  }
  .saved-rule-row--compact .saved-rule-target {
    grid-column: 2;
  }
  .saved-rule-row--compact .saved-rule-metadata {
    grid-column: 3;
    grid-row: 1;
  }
  .saved-rule-row--compact .saved-rule-mode {
    grid-column: 4;
    grid-row: 1;
  }
  .saved-rule-row--compact .saved-rule-remove {
    grid-column: 5;
    grid-row: 1;
  }
  .saved-rule-row--compact .rule-checkbox {
    grid-row: 1;
  }
  .saved-rule-row--compact .saved-rule-reason {
    grid-column: 2 / 4;
    grid-row: 2;
  }
  .saved-rules--compact .saved-rule-form {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) 8rem auto;
  }
  .saved-rules--compact .saved-rule-form > label:nth-child(-n + 2) {
    grid-column: auto;
  }
}
@media (pointer: coarse) {
  .saved-rules--compact {
    --saved-rule-action-width: 2.5rem;
  }
  .saved-rules--compact .saved-rules-heading :deep(button),
  .saved-rules--compact .saved-rule-remove :deep(button) {
    min-width: 2.5rem;
    min-height: 2.5rem;
  }
}
</style>
