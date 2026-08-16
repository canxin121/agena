<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import {
  approvalModelFromSettingsResponse,
  buildApprovalModelSettingsPatch,
  sameServerModelIdentity,
} from '@/lib/serverModelSettings'
import {
  defaultModeValue,
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
  useModelSelectionCatalog,
  type ModelModeOption,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
type PermissionScope = 'workspace' | 'global'

type PermissionRule = {
  id: number
  action_key: string
  subject_kind: string
  tool_name?: string | null
  qualifier?: string | null
  path_access_kind?: string | null
  target_path?: string | null
  network_target?: string | null
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

const toasts = useToastsStore()
const modelSelectionCatalog = useModelSelectionCatalog()

const loading = ref(false)
const error = ref('')
const rules = ref<PermissionRule[]>([])
const hasMore = ref(false)

const createBusy = ref(false)
const createError = ref('')
const newToolName = ref('')
const newQualifier = ref('')
const newMode = ref<PermissionMode>('ask')
const newScope = ref<PermissionScope>('workspace')

const approvalLoading = ref(false)
const approvalSaveBusy = ref(false)
const approvalError = ref('')
const approvalModelKey = ref('')
const approvalThinkingMode = ref('')
const approvalSpeedMode = ref('')

const modeOptions = [
  { value: 'allow', label: 'Allow', description: 'Approve matching tool calls.' },
  { value: 'auto', label: 'Auto', description: 'Let Agena evaluate matching calls automatically.' },
  { value: 'ask', label: 'Ask', description: 'Request confirmation before running.' },
  { value: 'deny', label: 'Deny', description: 'Block matching tool calls.' },
]

const scopeOptions = [
  { value: 'workspace', label: 'Workspace', description: 'Apply only to this workspace.' },
  { value: 'global', label: 'Global', description: 'Apply across all workspaces.' },
]

const canCreate = computed(() => !createBusy.value && newToolName.value.trim().length > 0)

const approvalModelOptions = computed(() => {
  const options: Array<{ value: string; label: string; description: string }> = []
  for (const provider of modelSelectionCatalog.providers.value) {
    for (const model of provider.models) {
      const adapter = String(model.adapter_id || '').trim()
      const value = encodeModelSelectionKey({ provider: provider.id, adapter, model: model.id })
      if (!value) continue
      options.push({
        value,
        label: String(model.display_name || model.id),
        description: [provider.id, adapter, model.id].filter(Boolean).join(' / '),
      })
    }
  }
  return options.sort((left, right) =>
    `${left.description}/${left.label}`.localeCompare(`${right.description}/${right.label}`),
  )
})

const selectedApprovalIdentity = computed(() => parseModelSlug(approvalModelKey.value))
const selectedApprovalModel = computed(() => {
  const selection = selectedApprovalIdentity.value
  return modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
})

function withSelectedMode(options: ModelModeOption[], selected: string): ModelModeOption[] {
  const value = String(selected || '').trim()
  if (!value || options.some((option) => option.value === value)) return options
  return [...options, { value, label: value, description: '', isDefault: false }]
}

const approvalThinkingOptions = computed(() =>
  withSelectedMode(thinkingModeOptionsForModel(selectedApprovalModel.value), approvalThinkingMode.value),
)
const approvalSpeedOptions = computed(() =>
  withSelectedMode(speedModeOptionsForModel(selectedApprovalModel.value), approvalSpeedMode.value),
)

function chooseApprovalModel(value: string) {
  approvalModelKey.value = value
  const selection = parseModelSlug(value)
  const model = modelSelectionCatalog.modelMetaFor(selection.provider, selection.model, selection.adapter)
  approvalThinkingMode.value = defaultModeValue(thinkingModeOptionsForModel(model))
  approvalSpeedMode.value = defaultModeValue(speedModeOptionsForModel(model))
  approvalError.value = ''
}

const sortedRules = computed(() =>
  [...rules.value].sort((a, b) => {
    const revokedOrder = Number(Boolean(a.revoked_at)) - Number(Boolean(b.revoked_at))
    return revokedOrder || b.id - a.id
  }),
)

function ruleTitle(rule: PermissionRule): string {
  if (rule.subject_kind === 'tool' && rule.tool_name) {
    return rule.qualifier ? `${rule.tool_name} · ${rule.qualifier}` : rule.tool_name
  }
  if (rule.subject_kind === 'path_access') {
    return `${rule.path_access_kind || 'path'} · ${rule.target_path || ''}`
  }
  if (rule.subject_kind === 'network_access') return rule.network_target || 'Network access'
  return rule.action_key
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const data = await apiJson<PermissionRulePage>('/api/v1/permission-rules?limit=200')
    rules.value = Array.isArray(data?.items) ? data.items : []
    hasMore.value = data?.page?.has_more === true
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    rules.value = []
    hasMore.value = false
  } finally {
    loading.value = false
  }
}

async function refreshApprovalModel() {
  approvalLoading.value = true
  approvalError.value = ''
  try {
    const [settingsResponse] = await Promise.all([
      apiJson<JsonValue>('/api/v1/settings?source=effective&path=permission.approval_model'),
      modelSelectionCatalog.loadProvidersAndModels(),
    ])
    const approval = approvalModelFromSettingsResponse(settingsResponse)
    approvalModelKey.value = approval
      ? encodeModelSelectionKey({
          provider: approval.identity.provider,
          adapter: approval.identity.adapter,
          model: approval.identity.model,
        })
      : ''
    approvalThinkingMode.value = approval?.modes.thinkingMode || ''
    approvalSpeedMode.value = approval?.modes.speedMode || ''
  } catch (err) {
    approvalError.value = err instanceof Error ? err.message : String(err)
  } finally {
    approvalLoading.value = false
  }
}

async function refreshAll() {
  await Promise.all([refresh(), refreshApprovalModel()])
}

async function saveApprovalModel() {
  if (approvalSaveBusy.value) return
  const desired = selectedApprovalIdentity.value
  const hasSelection = Boolean(desired.provider && desired.model)
  const desiredThinkingMode = hasSelection ? approvalThinkingMode.value.trim() : ''
  const desiredSpeedMode = hasSelection ? approvalSpeedMode.value.trim() : ''
  approvalSaveBusy.value = true
  approvalError.value = ''
  try {
    await apiJson('/api/v1/settings', {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        buildApprovalModelSettingsPatch(hasSelection ? desired : null, {
          ...(desiredThinkingMode ? { thinkingMode: desiredThinkingMode } : {}),
          ...(desiredSpeedMode ? { speedMode: desiredSpeedMode } : {}),
        }),
      ),
    })
    await refreshApprovalModel()
    const applied = selectedApprovalIdentity.value
    if (
      hasSelection !== Boolean(applied.provider && applied.model) ||
      (hasSelection && !sameServerModelIdentity(applied, desired)) ||
      (hasSelection && approvalThinkingMode.value.trim() !== desiredThinkingMode) ||
      (hasSelection && approvalSpeedMode.value.trim() !== desiredSpeedMode)
    ) {
      throw new Error('The server accepted the update but did not apply the automatic approval model.')
    }
    toasts.push('success', hasSelection ? 'Automatic approval model updated' : 'Automatic approval model cleared')
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    approvalError.value = message
    toasts.push('error', message)
  } finally {
    approvalSaveBusy.value = false
  }
}

async function createRule() {
  if (!canCreate.value) return
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
        scope: newScope.value,
      }),
    })
    newToolName.value = ''
    newQualifier.value = ''
    toasts.push('success', 'Permission rule created')
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
  try {
    await apiJson(`/api/v1/permission-rules/${id}`, { method: 'DELETE' })
    toasts.push('success', 'Permission rule removed')
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  }
}

onMounted(() => {
  void refreshAll()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">Permissions</div>
        <div class="mt-1 text-sm text-muted-foreground">Persistent rules Agena applies to tool approval decisions.</div>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing permission rules' : 'Refresh permission rules'"
        :aria-label="loading ? 'Refreshing permission rules' : 'Refresh permission rules'"
        :disabled="loading || approvalLoading"
        @click="refreshAll"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading || approvalLoading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <section class="grid gap-3 border-y border-border/60 py-4">
      <div>
        <h2 class="text-sm font-medium">Automatic approval model</h2>
        <p class="mt-1 text-xs text-muted-foreground">
          Used only to classify permission requests in Auto mode. Without a dedicated model, Agena falls back to the run
          or runtime default model.
        </p>
      </div>
      <div class="grid gap-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] lg:items-end">
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Model</span>
          <OptionPicker
            :model-value="approvalModelKey"
            :options="approvalModelOptions"
            title="Automatic approval model"
            empty-label="Use run/default model"
            placeholder="Select a configured model"
            search-placeholder="Search configured models..."
            :include-empty="true"
            :disabled="approvalLoading || approvalSaveBusy"
            monospace
            @update:model-value="chooseApprovalModel"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Thinking</span>
          <OptionPicker
            v-model="approvalThinkingMode"
            :options="approvalThinkingOptions"
            title="Approval thinking mode"
            empty-label="Model default"
            :include-empty="true"
            :disabled="approvalLoading || approvalSaveBusy || !approvalModelKey"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">Speed</span>
          <OptionPicker
            v-model="approvalSpeedMode"
            :options="approvalSpeedOptions"
            title="Approval speed mode"
            empty-label="Model default"
            :include-empty="true"
            :disabled="approvalLoading || approvalSaveBusy || !approvalModelKey"
          />
        </label>
        <Button class="h-10" :disabled="approvalLoading || approvalSaveBusy" @click="saveApprovalModel">
          <RiSave3Line class="mr-2 h-4 w-4" />
          {{ approvalSaveBusy ? 'Saving...' : 'Save model' }}
        </Button>
      </div>
      <div v-if="modelSelectionCatalog.catalogError.value" class="break-words text-xs text-destructive">
        {{ modelSelectionCatalog.catalogError.value }}
      </div>
      <div v-if="approvalError" class="break-words text-xs text-destructive">{{ approvalError }}</div>
    </section>

    <div class="grid gap-3 border-b border-border/60 pb-4">
      <div class="text-sm font-medium">Create tool rule</div>
      <div class="grid gap-3 sm:grid-cols-2">
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">Tool name</span>
          <Input
            v-model="newToolName"
            placeholder="shell"
            :disabled="createBusy"
            class="h-10 font-mono"
            @keydown.enter="createRule"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">Qualifier</span>
          <Input
            v-model="newQualifier"
            placeholder="Optional command or operation"
            :disabled="createBusy"
            class="h-10 font-mono"
            @keydown.enter="createRule"
          />
        </label>
      </div>
      <div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end">
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">Mode</span>
          <OptionPicker
            v-model="newMode"
            :options="modeOptions"
            title="Permission mode"
            :include-empty="false"
            :disabled="createBusy"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">Scope</span>
          <OptionPicker
            v-model="newScope"
            :options="scopeOptions"
            title="Permission scope"
            :include-empty="false"
            :disabled="createBusy"
          />
        </label>
        <Button class="h-10" :disabled="!canCreate" @click="createRule">
          {{ createBusy ? 'Creating...' : 'Create rule' }}
        </Button>
      </div>
      <div v-if="createError" class="break-words text-xs text-destructive">{{ createError }}</div>
    </div>

    <div class="grid gap-3">
      <div v-if="loading" class="text-sm text-muted-foreground">Loading permission rules...</div>
      <div
        v-else-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-else-if="sortedRules.length === 0" class="text-sm text-muted-foreground">
        No permission rules configured.
      </div>

      <div v-else class="space-y-2">
        <div
          v-for="rule in sortedRules"
          :key="rule.id"
          class="flex items-center justify-between gap-3 rounded-md border border-border/60 bg-background/50 px-3 py-2.5"
          :class="rule.revoked_at ? 'opacity-60' : ''"
        >
          <div class="min-w-0">
            <div class="break-words font-mono text-sm font-semibold">{{ ruleTitle(rule) }}</div>
            <div class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
              <span>#{{ rule.id }}</span>
              <span>mode: {{ rule.mode }}</span>
              <span>scope: {{ rule.scope }}</span>
              <span>source: {{ rule.source }}</span>
              <span v-if="rule.revoked_at" class="text-destructive">revoked</span>
            </div>
          </div>

          <ConfirmPopover
            title="Remove permission rule?"
            :description="ruleTitle(rule)"
            confirm-text="Remove"
            cancel-text="Cancel"
            variant="destructive"
            @confirm="removeRule(rule.id)"
          >
            <IconButton
              variant="outline"
              size="sm"
              class="shrink-0 text-destructive"
              tooltip="Remove rule"
              aria-label="Remove rule"
            >
              <RiDeleteBinLine class="h-4 w-4" />
            </IconButton>
          </ConfirmPopover>
        </div>
      </div>
      <div v-if="hasMore" class="text-xs text-muted-foreground">
        More than 200 rules exist. Refine or manage older rules through the CLI.
      </div>
    </div>
  </div>
</template>
