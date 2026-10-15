<script setup lang="ts">
import { computed, ref, watch } from 'vue'
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

const props = defineProps<{ scope: PermissionScope }>()
const toasts = useToastsStore()
const chat = useChatStore()
const directory = useDirectoryStore()
const directorySessions = useDirectorySessionStore()

const rules = ref<PermissionRule[]>([])
const hasMore = ref(false)
const selectedIds = ref<Set<number>>(new Set())

const createBusy = ref(false)
const createError = ref('')
const newToolName = ref('')
const newQualifier = ref('')
const newMode = ref<PermissionMode>('ask')
const bulkMode = ref<PermissionMode>('ask')
const bulkBusy = ref(false)
const rowBusy = ref<number | null>(null)
const query = ref('')
const stateFilter = ref('all')
const stateOptions = computed(() => [
  { value: 'all', label: st('All rules') },
  { value: 'active', label: st('Active') },
  { value: 'revoked', label: st('revoked') },
])
const activeSessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
const activeWorkspaceId = computed(() => {
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
defineExpose({ refresh })
watch(
  () => [props.scope, activeWorkspaceId.value, activeSessionId.value],
  () => {
    clearSelection()
    createError.value = ''
  },
)

async function setRuleMode(rule: PermissionRule, mode: string) {
  if (!canRemoveRule(rule) || rule.revoked_at || rowBusy.value !== null || bulkBusy.value || mode === rule.mode) return
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

async function applyBulkOperation(action: 'delete' | 'set_mode') {
  const targetRules = action === 'set_mode' ? selectedActiveRules.value : selectedRules.value
  if (!targetRules.length || bulkBusy.value || rowBusy.value !== null) return
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
  <PermissionSettingsCard
    :title="$st('Saved approval rules')"
    :description="$st('Rules remembered from approvals. Changes here take effect immediately.')"
    :count="sortedRules.length"
    :icon="RiShieldCheckLine"
  >
    <template #actions>
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
    <p v-if="scope === 'workspace' && !activeWorkspaceId" class="mb-3 text-xs text-muted-foreground">
      {{ $st('Select a workspace to view and manage its permission rules.') }}
    </p>
    <div v-if="sortedRules.length" class="mb-3 flex flex-wrap items-center gap-2">
      <SearchInput
        v-model="query"
        class="min-w-0 flex-1 basis-48"
        :placeholder="$st('Search rules…')"
        :input-aria-label="$st('Search saved approval rules')"
        :show-search-button="false"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
      />
      <div class="w-36">
        <OptionPicker
          v-model="stateFilter"
          :options="stateOptions"
          :include-empty="false"
          :title="$st('Rule status')"
        />
      </div>
      <label v-if="selectableRules.length" class="flex items-center gap-2 text-xs text-muted-foreground"
        ><input
          type="checkbox"
          class="accent-primary"
          :checked="allSelected"
          :indeterminate="selectedRules.length > 0 && !allSelected"
          :disabled="bulkBusy || rowBusy !== null"
          :aria-label="$st('Select all permission rules')"
          @change="toggleAllSelection"
        />{{ $st('Select all') }}</label
      >
    </div>
    <div
      v-if="selectedRules.length"
      class="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-primary/20 bg-primary/5 p-2.5"
    >
      <span class="text-xs font-medium">{{ $st('{count} selected', { count: selectedRules.length }) }}</span>
      <Button variant="ghost" size="sm" :disabled="bulkBusy" @click="clearSelection">{{ $st('Select none') }}</Button>
      <div class="flex flex-1 flex-wrap items-center justify-end gap-2">
        <div class="w-32">
          <OptionPicker
            v-model="bulkMode"
            :options="modeOptions"
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
      v-if="error"
      role="alert"
      class="mb-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <p v-if="loading && !rules.length" role="status" class="py-3 text-sm text-muted-foreground">
      {{ $st('Loading permission rules...') }}
    </p>
    <p v-else-if="!sortedRules.length && !error" class="py-3 text-sm text-muted-foreground">
      {{ $st('No permission rules configured.') }}
    </p>
    <div v-else-if="!filteredRules.length && !error" class="rounded-lg bg-muted/20 px-4 py-5 text-center">
      <p class="text-sm text-muted-foreground">{{ $st('No matching rules.') }}</p>
      <Button variant="ghost" size="sm" class="mt-1" @click="clearFilters">{{ $st('Clear filters') }}</Button>
    </div>
    <div class="grid gap-2">
      <div
        v-for="rule in filteredRules"
        :key="rule.id"
        class="saved-rule-row rounded-lg border border-border/50 bg-background/20 p-3"
        :class="rule.revoked_at ? 'opacity-60' : ''"
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
          <div class="break-all font-mono text-xs font-medium">{{ ruleTitle(rule) }}</div>
          <p v-if="rule.reason" class="mt-1 line-clamp-2 text-xs text-muted-foreground" :title="rule.reason">
            {{ rule.reason }}
          </p>
          <div class="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
            <span>{{ scopeLabel(rule.scope) }}</span
            ><span aria-hidden="true">·</span><span>#{{ rule.id }}</span
            ><span v-if="rule.source" :title="$st('Source')">{{ rule.source }}</span
            ><span v-if="rule.revoked_at" class="text-destructive">{{ $st('revoked') }}</span>
          </div>
        </div>
        <div class="saved-rule-mode">
          <OptionPicker
            v-if="canRemoveRule(rule) && !rule.revoked_at"
            :model-value="rule.mode"
            :options="modeOptions"
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
    <details v-if="canEdit" class="group mt-3 border-t border-border/50 pt-3">
      <summary
        class="flex cursor-pointer list-none items-center gap-2 rounded-md py-1 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <RiAddLine class="h-4 w-4" />{{ $st('Create tool rule')
        }}<RiArrowDownSLine class="ml-auto h-4 w-4 transition-transform group-open:rotate-180" />
      </summary>
      <form class="saved-rule-form mt-3" @submit.prevent="createRule">
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
        <Button type="submit" variant="outline" size="sm" class="h-9" :disabled="!canCreate">{{
          createBusy ? $st('Creating…') : $st('Create rule')
        }}</Button>
      </form>
      <p v-if="createError" class="mt-2 break-words text-xs text-destructive" role="alert">{{ createError }}</p>
    </details>
  </PermissionSettingsCard>
</template>

<style scoped>
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
</style>
