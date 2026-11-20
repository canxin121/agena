<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, useId, watch } from 'vue'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiSearchLine, RiShieldCheckLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import PermissionBulkActions from './permissions/PermissionBulkActions.vue'
import PermissionHelp from './permissions/PermissionHelp.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'
import { useChatStore } from '@/stores/chat'
import { getSession } from '@/stores/chat/api'
import { lastSessionId } from '@/lib/lastSession'
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

const props = defineProps<{ scope: PermissionScope; sessionId?: string; workspaceId?: number | null }>()
const toasts = useToastsStore()
const chat = useChatStore()

const rules = ref<PermissionRule[]>([])
const hasMore = ref(false)
const selectedIds = ref<Set<number>>(new Set())

const createBusy = ref(false)
const createError = ref('')
const createExpanded = ref(false)
const createFormId = useId()
const createForm = ref<HTMLFormElement | null>(null)
const headerActions = ref<HTMLElement | null>(null)
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
  return workspaceId(props.workspaceId)
})
const workspaceRuleTarget = computed(() => (props.scope === 'workspace' ? activeWorkspaceId.value : null))
const canEdit = computed(
  () => props.scope === 'global' || (props.scope === 'workspace' && activeWorkspaceId.value !== null),
)
function scopeLabel(scope: string) {
  if (scope === 'global') return st('Global')
  if (scope === 'workspace') return st('Workspace')
  if (scope === 'session') return st('Session')
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
  if (rule.subject_kind === 'network_access') return rule.network_target || st('Network access')
  return rule.action_key
}

const { loading, error, refresh } = usePaneRead(
  async (signal) => {
    const workspace = workspaceRuleTarget.value
    if (props.scope === 'workspace' && workspace === null) return {}
    const query = new URLSearchParams({ limit: String(PERMISSION_RULE_BATCH_SIZE) })
    if (workspace !== null) query.set('workspace_id', String(workspace))
    const data = await apiJson<PermissionRulePage>(`/api/v1/permission-rules?${query}`, { signal })
    return workspace === workspaceRuleTarget.value ? data : null
  },
  (data) => {
    if (!data) return
    rules.value = Array.isArray(data?.items) ? data.items : []
    hasMore.value = data?.page?.has_more === true
  },
)
defineExpose({ refresh, busy })
watch(workspaceRuleTarget, () => {
  rules.value = []
  hasMore.value = false
  void refresh()
})
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
    await writeRuleBatch([rule.id], { action: 'set_mode', mode }, ruleBatchTarget())
    await refresh()
  } catch (reason) {
    toasts.push('error', reason instanceof Error ? reason.message : String(reason))
  } finally {
    rowBusy.value = null
  }
}

function ruleBatchTarget() {
  const scope = props.scope
  return {
    scope,
    ...(scope === 'session' && activeSessionId.value !== null ? { session_id: activeSessionId.value } : {}),
    ...(scope === 'workspace' && workspaceRuleTarget.value !== null ? { workspace_id: workspaceRuleTarget.value } : {}),
  }
}

function writeRuleBatch(
  ids: number[],
  operation: { action: 'delete' } | { action: 'set_mode'; mode: string },
  target: ReturnType<typeof ruleBatchTarget>,
) {
  return apiJson<{ affected?: PermissionRule[]; missing_ids?: number[] }>('/api/v1/permission-rules/batch', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ rule_ids: ids, ...target, operation }),
  })
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
  await nextTick()
  if (createExpanded.value) createForm.value?.querySelector<HTMLInputElement>('input:not(:disabled)')?.focus()
  else headerActions.value?.querySelector<HTMLButtonElement>('button[aria-controls]')?.focus()
}

async function applyBulkOperation(action: 'delete' | 'set_mode') {
  const targetRules = action === 'set_mode' ? selectedActiveRules.value : selectedRules.value
  if (!targetRules.length || bulkBusy.value || rowBusy.value !== null) return
  if (action === 'set_mode' && !bulkModeOptions.value.some((option) => option.value === bulkMode.value)) return
  const ruleIds = targetRules.map((rule) => rule.id)
  const target = ruleBatchTarget()
  const operation = action === 'delete' ? { action } : { action, mode: bulkMode.value }
  bulkBusy.value = true
  try {
    let affectedCount = 0
    let missingCount = 0
    for (let offset = 0; offset < ruleIds.length; offset += PERMISSION_RULE_BATCH_SIZE) {
      const response = await writeRuleBatch(
        ruleIds.slice(offset, offset + PERMISSION_RULE_BATCH_SIZE),
        operation,
        target,
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
        ...ruleBatchTarget(),
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
    if (props.scope === 'workspace') {
      const result = await writeRuleBatch([id], { action: 'delete' }, ruleBatchTarget())
      if (result.missing_ids?.length) toasts.push('info', st('Some selected rules had changed and were skipped.'))
      else toasts.push('success', st('Permission rule removed'))
    } else {
      await apiJson(`/api/v1/permission-rules/${id}`, { method: 'DELETE' })
      toasts.push('success', st('Permission rule removed'))
    }
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    rowBusy.value = null
  }
}
</script>

<template>
  <section class="saved-rules" :aria-label="$st('Saved approval rules')" data-saved-rules>
    <header class="saved-rules-heading" :class="{ 'is-editable': canEdit }">
      <h2 class="saved-rules-title">
        <RiShieldCheckLine class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
        <span class="min-w-0">{{ $st('Saved approval rules') }}</span>
        <span v-if="sortedRules.length" class="saved-rules-count">{{ sortedRules.length }}</span>
      </h2>
      <p
        v-if="canEdit"
        class="saved-rules-immediate"
        :title="$st('Rules remembered from approvals. Changes here take effect immediately.')"
      >
        {{ $st('Applied immediately') }}
      </p>
      <div ref="headerActions" class="saved-rules-actions">
        <Button
          v-if="canEdit"
          variant="outline"
          size="sm"
          class="saved-rules-create-trigger gap-1.5 whitespace-normal px-2.5 text-[13px]"
          :aria-label="$st('Create tool rule')"
          :aria-expanded="createExpanded"
          :aria-controls="createFormId"
          :disabled="createBusy"
          @click="toggleCreateForm"
        >
          <RiAddLine class="h-4 w-4 shrink-0" aria-hidden="true" />
          {{ $st('Create tool rule') }}
        </Button>
        <IconButton
          variant="ghost"
          size="md"
          :tooltip="$st('Refresh permission rules')"
          :aria-label="$st('Refresh permission rules')"
          :disabled="loading"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
        </IconButton>
      </div>
    </header>

    <div class="saved-rules-body">
      <form
        v-if="canEdit"
        v-show="createExpanded"
        :id="createFormId"
        ref="createForm"
        class="saved-rule-create"
        @submit.prevent="createRule"
      >
        <div class="saved-rule-form-fields">
          <label class="saved-rule-field">
            <span>{{ $st('Tool name') }}</span>
            <Input
              v-model="newToolName"
              placeholder="agena.shell.exec"
              :disabled="createBusy"
              class="h-8 font-mono text-[13px]"
            />
          </label>
          <label class="saved-rule-field">
            <span>{{ $st('Qualifier') }}</span>
            <Input
              v-model="newQualifier"
              :placeholder="$st('Optional command or operation')"
              :disabled="createBusy"
              class="h-8 font-mono text-[13px]"
            />
          </label>
          <label class="saved-rule-field">
            <span>{{ $st('Mode') }}</span>
            <OptionPicker
              v-model="newMode"
              :options="modeOptions"
              :title="$st('Permission mode')"
              :include-empty="false"
              :disabled="createBusy"
              size="sm"
            />
          </label>
          <div class="saved-rule-create-actions">
            <Button type="button" variant="ghost" size="sm" :disabled="createBusy" @click="toggleCreateForm">
              {{ $st('Cancel') }}
            </Button>
            <Button type="submit" variant="default" size="sm" :disabled="!canCreate">
              {{ createBusy ? $st('Creating…') : $st('Create rule') }}
            </Button>
          </div>
        </div>
        <p v-if="createError" class="mt-2 break-words text-xs text-destructive" role="alert">{{ createError }}</p>
      </form>

      <p v-if="error || targetWorkspaceError" class="saved-rules-message" role="alert">
        {{ error || targetWorkspaceError }}
      </p>
      <p v-if="scope === 'workspace' && !activeWorkspaceId" class="saved-rules-message">
        {{ $st('Select a workspace to view and manage its permission rules.') }}
      </p>
      <p v-else-if="loading && !rules.length" class="saved-rules-message" role="status">
        {{ $st('Loading permission rules...') }}
      </p>
      <p v-else-if="!sortedRules.length && !error && !targetWorkspaceError" class="saved-rules-message">
        {{ $st('No permission rules configured.') }}
      </p>

      <template v-if="sortedRules.length">
        <div class="saved-rules-controls" data-saved-rules-controls>
          <div class="relative min-w-0">
            <SearchInput
              v-model="query"
              :placeholder="$st('Search rules…')"
              :input-aria-label="$st('Search saved approval rules')"
              input-class="h-8 px-8 text-[13px]"
              clear-button-class="end-1 right-auto"
              :show-search-button="false"
              :clear-aria-label="$st('Clear search')"
              :clear-title="$st('Clear search')"
            />
            <RiSearchLine
              class="pointer-events-none absolute start-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
          </div>
          <OptionPicker
            v-model="stateFilter"
            :options="stateOptions"
            :include-empty="false"
            :title="$st('Rule status')"
            size="sm"
          />
          <PermissionHelp
            v-if="sortedRules.some((rule) => rule.subject_kind === 'network_access')"
            :text="
              $st(
                'Network decisions accept allow or deny and literal IP targets such as 192.0.2.10:443. Use tcp://, udp:// or icmp:// for a protocol; bare IP covers configured protocols and ports. Configure CIDRs in network settings.',
              )
            "
          />
        </div>

        <PermissionBulkActions
          v-if="selectedRules.length"
          v-model="bulkMode"
          class="saved-rules-bulk"
          :count="selectedRules.length"
          :options="bulkModeOptions"
          :disabled="bulkBusy || rowBusy !== null"
          :mode-disabled="!selectedActiveRules.length"
          :clear-disabled="bulkBusy"
          @apply="applyBulkOperation('set_mode')"
          @clear="clearSelection"
        >
          <template #remove>
            <ConfirmPopover
              :title="$st('Remove selected permission rules?')"
              :description="$st('Remove {count} selected permission rules?', { count: selectedRules.length })"
              :confirm-text="$st('Remove selected')"
              :cancel-text="$st('Cancel')"
              variant="destructive"
              :confirm-disabled="bulkBusy || rowBusy !== null"
              @confirm="applyBulkOperation('delete')"
            >
              <IconButton
                variant="ghost-destructive"
                size="md"
                class="pointer-coarse:w-9"
                :aria-label="$st('Remove selected')"
                :tooltip="$st('Remove selected')"
                :disabled="bulkBusy || rowBusy !== null"
              >
                <RiDeleteBinLine class="h-4 w-4" aria-hidden="true" />
              </IconButton>
            </ConfirmPopover>
          </template>
        </PermissionBulkActions>

        <div v-if="filteredRules.length" class="saved-rules-results" :class="{ 'is-editable': canEdit }">
          <div class="saved-rules-columns">
            <label v-if="selectableRules.length" class="saved-rules-select-all" :title="$st('Select all')">
              <input
                type="checkbox"
                class="accent-primary"
                :checked="allSelected"
                :indeterminate="selectedRules.length > 0 && !allSelected"
                :disabled="bulkBusy || rowBusy !== null"
                :aria-label="$st('Select all permission rules')"
                @change="toggleAllSelection"
              />
              <span>{{ $st('Select all') }}</span>
            </label>
            <span class="saved-rules-column saved-rule-target" aria-hidden="true">{{ $st('Rule target') }}</span>
            <span class="saved-rules-column saved-rule-scope" aria-hidden="true">{{ $st('Scope') }}</span>
            <span class="saved-rules-column saved-rule-mode" aria-hidden="true">{{ $st('Mode') }}</span>
            <span v-if="canEdit" class="saved-rules-column saved-rule-remove" aria-hidden="true">
              {{ $st('Actions') }}
            </span>
          </div>
          <ul class="saved-rules-list" role="list">
            <li v-for="rule in filteredRules" :key="rule.id" class="saved-rule-row" :data-saved-rule-id="rule.id">
              <label v-if="canRemoveRule(rule)" class="saved-rule-select">
                <input
                  type="checkbox"
                  class="accent-primary"
                  :checked="selectedIds.has(rule.id)"
                  :disabled="bulkBusy || rowBusy !== null"
                  :aria-label="$st('Select permission rule {id}', { id: rule.id })"
                  @change="toggleRuleSelection(rule.id, ($event.target as HTMLInputElement).checked)"
                />
              </label>
              <div class="saved-rule-target">
                <code class="font-medium">{{ ruleTitle(rule) }}</code>
                <p v-if="rule.reason" class="saved-rule-detail">{{ rule.reason }}</p>
              </div>
              <div class="saved-rule-scope">
                <span>{{ scopeLabel(rule.scope) }}</span>
                <span v-if="rule.revoked_at" class="ms-1.5 text-destructive">{{ $st('revoked') }}</span>
                <p class="saved-rule-detail">
                  <template v-if="rule.source">{{ rule.source }} <span aria-hidden="true">·</span> </template>
                  #{{ rule.id }}
                </p>
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
                <span v-else class="saved-rule-mode-label" :class="{ 'text-destructive': rule.mode === 'deny' }">
                  {{ modeOptions.find((option) => option.value === rule.mode)?.label || rule.mode }}
                </span>
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
                    size="md"
                    :tooltip="$st('Remove rule')"
                    :aria-label="$st('Remove rule')"
                    :disabled="bulkBusy || rowBusy !== null"
                  >
                    <RiDeleteBinLine class="h-4 w-4" aria-hidden="true" />
                  </IconButton>
                </ConfirmPopover>
              </div>
            </li>
          </ul>
        </div>
        <div v-else-if="!error" class="saved-rules-message flex flex-wrap items-center justify-between gap-1">
          <p>{{ $st('No matching rules.') }}</p>
          <Button variant="ghost" size="sm" @click="clearFilters">{{ $st('Clear filters') }}</Button>
        </div>
      </template>

      <p v-if="hasMore" class="text-xs text-muted-foreground">
        {{ $st('More than 200 rules exist. Refine or manage older rules through the CLI.') }}
      </p>
    </div>
  </section>
</template>

<style scoped>
@reference '../../style.css';

.saved-rules {
  @apply rounded-xl border border-border/75 bg-card/75;
  container: saved-rules / inline-size;
  min-width: 0;
  font-size: 0.8125rem;
}
.saved-rules-heading {
  @apply rounded-t-xl border-b border-border/65 bg-muted/18;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 0.375rem 0.75rem;
  min-height: 2.75rem;
  padding: 0.375rem 0.75rem;
}
.saved-rules-title {
  display: flex;
  align-items: center;
  gap: 0.4375rem;
  min-width: 0;
  font-size: 0.9375rem;
  font-weight: 650;
  line-height: 1.35;
  overflow-wrap: anywhere;
}
.saved-rules-count {
  @apply rounded-md bg-muted/50 px-1.5 py-0.5 text-xs font-medium tabular-nums text-muted-foreground;
}
.saved-rules-immediate {
  @apply text-xs text-muted-foreground;
}
.saved-rules-actions,
.saved-rule-create-actions {
  display: flex;
  align-items: center;
  justify-content: end;
  gap: 0.375rem;
  min-width: 0;
}
.saved-rules-actions {
  grid-column: -2;
}
.saved-rules-body {
  display: grid;
  gap: 0.625rem;
  min-width: 0;
  padding: 0.75rem;
}
.saved-rules-message {
  @apply rounded-lg px-3 py-2 text-muted-foreground;
  overflow-wrap: anywhere;
}
.saved-rules-message:not([role='alert']) {
  @apply bg-muted/25;
}
.saved-rules-message[role='alert'] {
  @apply border border-destructive/30 bg-destructive/5 text-destructive;
}
.saved-rules-controls {
  display: grid;
  grid-template-columns: minmax(0, 1fr) min(8.5rem, 43%);
  gap: 0.5rem;
  width: min(100%, 35rem);
}
.saved-rules-controls:has(.permission-help) {
  grid-template-columns: minmax(0, 1fr) min(8.5rem, 43%) auto;
}
.saved-rules-controls :deep(.truncate) {
  white-space: normal;
  overflow-wrap: anywhere;
}
.saved-rule-create {
  @apply rounded-lg border border-primary/20 bg-primary/5;
  padding: 0.625rem;
}
.saved-rule-form-fields {
  display: grid;
  align-items: end;
  gap: 0.5rem 0.625rem;
}
.saved-rule-field {
  display: grid;
  gap: 0.25rem;
  min-width: 0;
}
.saved-rule-field > span {
  @apply text-xs text-muted-foreground;
}
.saved-rule-create-actions {
  flex-wrap: wrap;
}
.saved-rules-results {
  @apply rounded-lg border border-border/60;
  --saved-rule-columns: minmax(0, 1fr) auto;
  --saved-rule-areas: 'target target' 'scope mode';
  min-width: 0;
}
.saved-rules-results.is-editable {
  --saved-rule-columns: 1.125rem minmax(0, 1fr) minmax(5.25rem, 28%) auto;
  --saved-rule-areas: 'selection target target target' '. scope mode actions';
}
.saved-rules-columns,
.saved-rule-row {
  display: grid;
  align-items: center;
  gap: 0.25rem 0.5rem;
  padding: 0.375rem 0.625rem;
}
.saved-rules-columns {
  @apply rounded-t-lg border-b border-border/60 bg-muted/25 text-xs text-muted-foreground;
}
.saved-rules-results:not(.is-editable) .saved-rules-columns,
.saved-rules-column {
  display: none;
}
.saved-rules-select-all,
.saved-rule-select {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-height: 1.75rem;
  cursor: pointer;
}
.saved-rule-select {
  grid-area: selection;
  align-self: start;
}
.saved-rules-select-all input,
.saved-rule-select input {
  width: 0.875rem;
  height: 0.875rem;
}
.saved-rule-row {
  grid-template-columns: var(--saved-rule-columns);
  grid-template-areas: var(--saved-rule-areas);
  overflow-wrap: anywhere;
}
.saved-rule-row + .saved-rule-row {
  @apply border-t border-border/50;
}
.saved-rule-row:has(input:checked) {
  @apply bg-primary/5;
}
.saved-rule-row:last-child {
  @apply rounded-b-lg;
}
.saved-rule-target {
  grid-area: target;
  min-width: 0;
}
.saved-rule-scope {
  grid-area: scope;
  min-width: 0;
}
.saved-rule-detail {
  @apply text-xs text-muted-foreground;
}
.saved-rule-mode {
  grid-area: mode;
  min-width: 0;
}
.saved-rule-mode-label {
  @apply inline-flex rounded-md bg-muted/35 px-2 py-1 text-xs font-medium;
}
.saved-rule-remove {
  grid-area: actions;
  justify-self: end;
  text-align: end;
}
@container saved-rules (max-width: 30rem) {
  .saved-rules-heading {
    grid-template-columns: minmax(0, 1fr) auto;
  }
  .saved-rules-heading.is-editable .saved-rules-title {
    grid-column: 1 / -1;
  }
}
@container saved-rules (min-width: 29rem) {
  .saved-rule-form-fields {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@container saved-rules (min-width: 42rem) {
  .saved-rules-results {
    --saved-rule-columns: minmax(0, 1fr) minmax(8rem, 0.35fr) 8rem;
    --saved-rule-areas: 'target scope mode';
  }
  .saved-rules-results.is-editable {
    --saved-rule-columns: 1.125rem minmax(0, 1fr) minmax(8rem, 0.35fr) 8rem 3.125rem;
    --saved-rule-areas: 'selection target scope mode actions';
  }
  .saved-rules-columns {
    grid-template-columns: var(--saved-rule-columns);
    grid-template-areas: var(--saved-rule-areas);
  }
  .saved-rules-results:not(.is-editable) .saved-rules-columns {
    display: grid;
  }
  .saved-rules-select-all {
    grid-area: selection;
  }
  .saved-rule-select {
    align-self: center;
  }
  .saved-rules-select-all > span {
    @apply sr-only;
  }
  .saved-rules-column {
    display: block;
    overflow-wrap: anywhere;
  }
  .saved-rule-form-fields {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr) 8rem auto;
  }
}
@media (pointer: coarse) {
  .saved-rules-actions :deep(button),
  .saved-rule-remove :deep(button) {
    min-height: 2.5rem;
  }
  .saved-rules-actions :deep(.oc-icon-button),
  .saved-rule-remove :deep(.oc-icon-button) {
    min-width: 2.5rem;
  }
}
</style>
