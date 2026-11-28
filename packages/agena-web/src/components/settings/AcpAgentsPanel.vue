<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import {
  RiAddLine,
  RiDeleteBinLine,
  RiInformationLine,
  RiRefreshLine,
  RiResetLeftLine,
  RiSettings3Line,
  RiShieldCheckLine,
  RiTerminalBoxLine,
} from '@remixicon/vue'

import AcpInstallGuideCard from '@/components/acp/AcpInstallGuideCard.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import SettingsCard from './SettingsCard.vue'
import SettingsDisclosureRow from './SettingsDisclosureRow.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsToggle from './SettingsToggle.vue'
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import {
  applyAcpAgentDraft,
  blankCustomAcpAgentDraft,
  buildAcpAgentEntryFromDraft,
  draftFromAcpAgent,
  findBuiltinAcpAgent,
  formatAcpCommandPreview,
  hasCustomOverride,
  isBuiltinAcpAgent,
  listAcpAgentIds,
  parseAcpConfigDocument,
  removeAcpAgentFromConfig,
  resolveAcpAgentSummary,
  serializeAcpConfigDocument,
  validateAcpAgentDraft,
  type AcpAgentDraft,
  type AcpAgentStatusKind,
  type AcpAgentValidationError,
  type AcpConfigDocument,
} from '@/lib/acpAgentsConfig'
import { listExternalAgents, useExternalAgentCatalog } from '@/features/sessions/api/nativeAgentSessions'
import { confirmAction } from '@/lib/appConfirm'
import {
  deleteRuntimeSetting,
  getRuntimeSettingLayer,
  setRuntimeSetting,
} from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'

type AgentRow =
  | {
      key: '__new__'
      isNew: true
      id: string
      name: string
      summary: string
      status: AcpAgentStatusKind
      disabled: boolean
      builtin: boolean
      overridden: boolean
      available: undefined
    }
  | {
      key: string
      isNew: false
      id: string
      name: string
      command: string
      summary: string
      status: AcpAgentStatusKind
      disabled: boolean
      builtin: boolean
      overridden: boolean
      available: boolean | undefined
    }

const { t } = useI18n()
const route = useRoute()
const toasts = useToastsStore()
const liveCatalog = useExternalAgentCatalog()

const doc = ref<AcpConfigDocument>({ agents: {} })
const revision = ref<string | undefined>()
const configPath = ref('')
const selectedId = ref<string | null>(null)
const creating = ref(false)
const draft = ref<AcpAgentDraft | null>(null)
const savedSnapshot = ref('')
const search = ref('')
const loading = ref(false)
const checkingAvailability = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
let generation = 0
let disposed = false

function availabilityFor(id: string | null): boolean | undefined {
  if (!id) return undefined
  return liveCatalog.value.find((entry) => entry.id === id)?.available
}

const allIds = computed(() => listAcpAgentIds(doc.value))
const summaries = computed(() =>
  allIds.value.map((id) => resolveAcpAgentSummary(id, doc.value.agents[id])),
)

const dirty = computed(() => draft.value !== null && JSON.stringify(draft.value) !== savedSnapshot.value)
const activeBuiltin = computed(() => (draft.value ? findBuiltinAcpAgent(draft.value.id.trim()) : undefined))
const activeEntry = computed(() => (selectedId.value ? doc.value.agents[selectedId.value] : undefined))

const selectedSummary = computed(() =>
  selectedId.value ? resolveAcpAgentSummary(selectedId.value, activeEntry.value) : null,
)
const selectedAvailability = computed(() =>
  !creating.value && selectedId.value && !draft.value?.disabled ? availabilityFor(selectedId.value) : undefined,
)

const filteredSummaries = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return summaries.value
  return summaries.value.filter(
    (entry) =>
      (entry.id === selectedId.value && dirty.value) ||
      [entry.id, entry.name, entry.command, ...entry.args].some((text) => text.toLowerCase().includes(q)),
  )
})

const rows = computed<AgentRow[]>(() => {
  const list: AgentRow[] = []
  if (creating.value && draft.value) {
    list.push({
      key: '__new__',
      isNew: true,
      id: draft.value.id.trim(),
      name: draft.value.name.trim() || draft.value.id.trim() || String(t('settings.acpAgents.newAgentTitle')),
      summary: effectivePreview.value || String(t('settings.acpAgents.newAgentSubtitle')),
      status: 'custom',
      disabled: draft.value.disabled,
      builtin: false,
      overridden: false,
      available: undefined,
    })
  }
  for (const entry of filteredSummaries.value) {
    list.push({
      key: entry.id,
      isNew: false,
      id: entry.id,
      name: entry.name,
      command: entry.command,
      summary: formatAcpCommandPreview(entry.command, entry.args) || entry.id,
      status: entry.status,
      disabled: entry.disabled,
      builtin: entry.builtin,
      overridden: entry.builtin && hasCustomOverride(doc.value.agents[entry.id]),
      available: entry.disabled ? undefined : availabilityFor(entry.id),
    })
  }
  return list
})

const validationErrors = computed(() => {
  if (!draft.value) return []
  return validateAcpAgentDraft(draft.value, Object.keys(doc.value.agents), creating.value)
})

const effectivePreview = computed(() => {
  if (!draft.value) return ''
  const id = draft.value.id.trim() || 'custom-agent'
  const built = buildAcpAgentEntryFromDraft(draft.value)
  const resolved = resolveAcpAgentSummary(id, built)
  return formatAcpCommandPreview(resolved.command, resolved.args)
})

const { canDiscard } = useSettingsDraftGuard({
  dirty: () => dirty.value,
  busy: () => saving.value,
  unload: () => dirty.value,
  dialog: () => ({ description: t('settings.acpAgents.unsavedWarning') }),
})

function isRowOpen(row: AgentRow): boolean {
  if (row.isNew) return creating.value
  return !creating.value && selectedId.value === row.id
}

function statusLabel(status: AcpAgentStatusKind): string {
  return String(t(`settings.acpAgents.status.${status}`))
}

function statusClass(status: AcpAgentStatusKind): string {
  if (status === 'disabled') return 'border-border/70 bg-muted/50 text-muted-foreground'
  if (status === 'overridden') return 'border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300'
  if (status === 'custom') return 'border-primary/30 bg-primary/10 text-primary'
  return 'border-border/70 bg-muted/40 text-muted-foreground'
}

function validationMessage(issue: AcpAgentValidationError): string {
  switch (issue.code) {
    case 'invalid_id':
      return String(t('settings.acpAgents.validation.invalidId'))
    case 'duplicate_id':
      return String(t('settings.acpAgents.validation.duplicateId'))
    case 'invalid_name':
      return String(t('settings.acpAgents.validation.invalidName'))
    case 'missing_command':
      return String(t('settings.acpAgents.validation.missingCommand'))
    case 'invalid_command':
      return String(t('settings.acpAgents.validation.invalidCommand'))
    case 'invalid_args':
      return String(t('settings.acpAgents.validation.invalidArgs'))
    case 'too_many_env':
      return String(t('settings.acpAgents.validation.tooManyEnv'))
    case 'invalid_env_key':
      return String(t('settings.acpAgents.validation.invalidEnvKey', { key: issue.key || '?' }))
    case 'duplicate_env_key':
      return String(t('settings.acpAgents.validation.duplicateEnvKey', { key: issue.key }))
    case 'invalid_env_value':
      return String(t('settings.acpAgents.validation.invalidEnvValue', { key: issue.key }))
    case 'invalid_writable_path':
      return String(t('settings.acpAgents.validation.invalidWritablePath', { path: issue.path }))
    case 'too_many_writable_paths':
      return String(t('settings.acpAgents.validation.tooManyWritablePaths'))
  }
}

function openExisting(id: string | null) {
  creating.value = false
  saveError.value = ''
  selectedId.value = id
  if (!id) {
    draft.value = null
    savedSnapshot.value = ''
    return
  }
  const next = draftFromAcpAgent(id, doc.value.agents[id])
  draft.value = next
  savedSnapshot.value = JSON.stringify(next)
}

async function toggleRow(row: AgentRow) {
  if (row.isNew) {
    await discardDraft()
    return
  }
  if (!creating.value && selectedId.value === row.id) {
    if (!(await canDiscard())) return
    openExisting(null)
    return
  }
  if (!(await canDiscard())) return
  openExisting(row.id)
}

async function startCreate() {
  if (creating.value) return
  if (!(await canDiscard())) return
  creating.value = true
  selectedId.value = null
  saveError.value = ''
  const next = blankCustomAcpAgentDraft()
  draft.value = next
  savedSnapshot.value = JSON.stringify(next)
}

function addEnvRow() {
  if (!draft.value) return
  draft.value.envRows.push({ key: '', value: '' })
}

function removeEnvRow(index: number) {
  if (!draft.value) return
  draft.value.envRows.splice(index, 1)
}

async function persistDocument(nextDoc: AcpConfigDocument, targetId: string | null, toastKey: string) {
  saving.value = true
  saveError.value = ''
  try {
    const serialized = serializeAcpConfigDocument(nextDoc)
    const options = {
      expected_revision: revision.value,
      validate: true,
      reload: true,
    }
    const response = serialized
      ? await setRuntimeSetting('acp', serialized, options, 'global')
      : await deleteRuntimeSetting('acp', options, 'global')
    if (disposed) return
    revision.value = response.after_revision ?? revision.value
    doc.value = nextDoc
    const nextOpenId = targetId && listAcpAgentIds(nextDoc).includes(targetId) ? targetId : null
    openExisting(nextOpenId)
    toasts.push('success', String(t(toastKey)))
    void listExternalAgents().catch(() => {})
  } catch (reason) {
    if (!disposed) {
      saveError.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (!disposed) saving.value = false
  }
}

async function saveDraft() {
  if (!draft.value || saving.value || validationErrors.value.length > 0) return
  const targetId = draft.value.id.trim()
  const nextDoc = applyAcpAgentDraft(doc.value, draft.value)
  await persistDocument(nextDoc, targetId, 'settings.acpAgents.savedToast')
}

async function discardDraft() {
  if (!(await canDiscard())) return
  saveError.value = ''
  if (creating.value) {
    creating.value = false
    openExisting(null)
    return
  }
  if (selectedId.value) openExisting(selectedId.value)
}

async function resetBuiltin(id: string) {
  if (!isBuiltinAcpAgent(id) || !hasCustomOverride(doc.value.agents[id]) || saving.value) return
  if (selectedId.value !== id && !(await canDiscard())) return
  const ok = await confirmAction({
    title: String(t('settings.acpAgents.resetConfirmTitle')),
    description: String(t('settings.acpAgents.resetConfirmDescription', { id })),
    confirmText: String(t('settings.acpAgents.resetAction')),
    cancelText: String(t('common.cancel')),
  })
  if (!ok) return
  const nextDoc = removeAcpAgentFromConfig(doc.value, id)
  await persistDocument(nextDoc, selectedId.value === id ? id : selectedId.value, 'settings.acpAgents.resetToast')
}

async function deleteCustomAgent(id: string) {
  if (isBuiltinAcpAgent(id) || saving.value) return
  if (selectedId.value !== id && !(await canDiscard())) return
  const ok = await confirmAction({
    title: String(t('settings.acpAgents.deleteConfirmTitle')),
    description: String(t('settings.acpAgents.deleteConfirmDescription', { id })),
    confirmText: String(t('common.delete')),
    cancelText: String(t('common.cancel')),
    variant: 'destructive',
  })
  if (!ok) return
  const nextDoc = removeAcpAgentFromConfig(doc.value, id)
  await persistDocument(nextDoc, selectedId.value === id ? null : selectedId.value, 'settings.acpAgents.deletedToast')
}

function applyNpxPreset(preset: { command: string; args: readonly string[] }) {
  if (!draft.value || saving.value) return
  draft.value.command = preset.command
  draft.value.argsText = preset.args.join('\n')
}

async function recheckAvailability() {
  if (checkingAvailability.value) return
  checkingAvailability.value = true
  try {
    await listExternalAgents()
  } catch {
    // Keep existing catalog state when refresh fails.
  } finally {
    if (!disposed) checkingAvailability.value = false
  }
}

function queryAgentId(): string | null {
  const raw = route.query.acpAgent
  const value = Array.isArray(raw) ? raw[0] : raw
  return typeof value === 'string' && value.trim() ? value.trim() : null
}

async function load() {
  const token = ++generation
  loading.value = true
  error.value = ''
  try {
    const [response] = await Promise.all([
      getRuntimeSettingLayer('global', 'acp'),
      listExternalAgents().catch(() => null),
    ])
    if (disposed || token !== generation) return
    doc.value = parseAcpConfigDocument(response.value)
    revision.value = response.revision
    configPath.value = response.config_path || ''
    const ids = listAcpAgentIds(doc.value)
    if (!creating.value) {
      const requested = queryAgentId()
      const nextId =
        requested && ids.includes(requested)
          ? requested
          : selectedId.value && ids.includes(selectedId.value)
            ? selectedId.value
            : null
      openExisting(nextId)
    }
  } catch (reason) {
    if (!disposed && token === generation) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (!disposed && token === generation) loading.value = false
  }
}

async function refresh() {
  if (!(await canDiscard())) return
  await load()
}

defineExpose({ refresh, dirty, saving })

watch(
  () => route.query.acpAgent,
  () => {
    const requested = queryAgentId()
    if (!requested || loading.value || creating.value || dirty.value) return
    if (allIds.value.includes(requested) && selectedId.value !== requested) {
      openExisting(requested)
    }
  },
)

onMounted(() => void load())
onBeforeUnmount(() => {
  disposed = true
})
</script>

<template>
  <section class="settings-stack">
    <div class="settings-toolbar rounded-lg border border-border/70 bg-card/30 p-2.5">
      <SearchInput
        v-model="search"
        class="settings-toolbar-search"
        :placeholder="t('settings.acpAgents.searchPlaceholder')"
        :input-aria-label="t('settings.acpAgents.searchPlaceholder')"
        :show-search-button="false"
      />
      <IconButton
        variant="ghost"
        size="sm"
        :disabled="loading || saving"
        :tooltip="t('common.refresh')"
        :aria-label="t('common.refresh')"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button size="sm" :disabled="loading || saving" @click="startCreate">
        <RiAddLine class="mr-1.5 h-4 w-4" />
        {{ t('settings.acpAgents.addCustom') }}
      </Button>
    </div>

    <div class="flex items-start gap-2 rounded-lg border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
      <RiInformationLine class="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
      <div class="min-w-0 space-y-0.5">
        <p>{{ t('settings.acpAgents.globalScopeNotice') }}</p>
        <p v-if="configPath" class="break-all font-mono text-[11px] opacity-80">{{ configPath }}</p>
      </div>
    </div>

    <p
      v-if="error"
      role="alert"
      class="break-words rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </p>

    <div
      v-if="loading && !summaries.length"
      role="status"
      class="rounded-xl border border-border/60 p-5 text-sm text-muted-foreground"
    >
      {{ $st('Loading…') }}
    </div>

    <div v-else class="grid min-w-0 gap-2">
      <SettingsDisclosureRow
        v-for="row in rows"
        :key="row.key"
        class="min-w-0 rounded-lg bg-card/30"
        body-class="p-3 lg:p-4"
        :disabled="loading || saving"
        :open="isRowOpen(row)"
        :label="row.name"
        :summary="row.summary"
        :tone="isRowOpen(row) && (dirty || row.isNew) ? 'dirty' : row.disabled ? 'disabled' : 'default'"
        @toggle="toggleRow(row)"
      >
        <template #leading>
          <span
            class="hidden h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/30 sm:flex"
          >
            <RiTerminalBoxLine class="h-4 w-4 text-muted-foreground" />
          </span>
        </template>

        <template #badges>
          <span
            v-if="isRowOpen(row) && dirty"
            class="shrink-0 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300"
          >
            {{ $st('Unsaved') }}
          </span>
          <span
            v-else-if="row.isNew"
            class="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary"
          >
            {{ $st('New') }}
          </span>
          <span
            class="shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] font-medium"
            :class="statusClass(row.status)"
          >
            {{ statusLabel(row.status) }}
          </span>
          <span
            v-if="!row.isNew && row.available !== undefined"
            class="inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] font-medium"
            :class="
              row.available
                ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300'
                : 'border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300'
            "
          >
            <span
              class="h-1.5 w-1.5 rounded-full"
              :class="row.available ? 'bg-emerald-500' : 'bg-amber-500'"
              aria-hidden="true"
            />
            {{ t(row.available ? 'settings.acpAgents.availability.ready' : 'settings.acpAgents.availability.notFound') }}
          </span>
        </template>

        <template #actions>
          <IconButton
            v-if="!row.isNew && row.overridden"
            variant="ghost"
            size="sm"
            :disabled="saving"
            :tooltip="t('settings.acpAgents.resetAction')"
            :aria-label="t('settings.acpAgents.resetAction')"
            @click="resetBuiltin(row.id)"
          >
            <RiResetLeftLine class="h-4 w-4" />
          </IconButton>
          <IconButton
            v-if="row.isNew || !row.builtin"
            variant="ghost"
            size="sm"
            :disabled="saving"
            :tooltip="row.isNew ? t('common.cancel') : t('common.delete')"
            :aria-label="row.isNew ? t('common.cancel') : t('common.delete')"
            @click="row.isNew ? discardDraft() : deleteCustomAgent(row.id)"
          >
            <RiDeleteBinLine class="h-4 w-4 text-destructive" />
          </IconButton>
        </template>

        <fieldset v-if="isRowOpen(row) && draft" :disabled="loading || saving" class="settings-stack">
          <AcpInstallGuideCard
            v-if="!row.isNew && selectedAvailability === false && selectedId && selectedSummary"
            :agent-id="selectedId"
            :agent-name="selectedSummary.name"
            :command="selectedSummary.command"
            :show-configure="false"
            :show-retry="true"
            :show-use-npx="selectedId === 'claude-code'"
            :retrying="checkingAvailability"
            @retry="recheckAvailability"
            @use-npx="applyNpxPreset"
          />

          <SettingsCard
            :title="t('settings.acpAgents.detailLabel')"
            :description="row.isNew ? t('settings.acpAgents.newAgentSubtitle') : draft.id"
            :icon="RiSettings3Line"
            heading="h3"
          >
            <div class="space-y-3">
              <SettingsRow
                :title="t('settings.acpAgents.fields.enabled')"
                :description="t('settings.acpAgents.fields.enabledDescription')"
                toggle
              >
                <SettingsToggle
                  :model-value="!draft.disabled"
                  :label="t('settings.acpAgents.fields.enabled')"
                  :disabled="saving"
                  @update:model-value="draft.disabled = !$event"
                />
              </SettingsRow>

              <SettingsRow
                v-if="row.isNew"
                :title="t('settings.acpAgents.fields.id')"
                :description="t('settings.acpAgents.fields.idDescription')"
              >
                <Input
                  v-model="draft.id"
                  class="font-mono"
                  :placeholder="t('settings.acpAgents.idPlaceholder')"
                  :aria-label="t('settings.acpAgents.fields.id')"
                  :disabled="saving"
                />
              </SettingsRow>

              <SettingsRow
                :title="t('settings.acpAgents.fields.name')"
                :description="t('settings.acpAgents.fields.nameDescription')"
              >
                <Input
                  v-model="draft.name"
                  :placeholder="activeBuiltin?.defaultName || draft.id || t('settings.acpAgents.namePlaceholder')"
                  :aria-label="t('settings.acpAgents.fields.name')"
                  :disabled="saving"
                />
              </SettingsRow>

              <SettingsRow
                :title="t('settings.acpAgents.fields.command')"
                :description="
                  activeBuiltin
                    ? t('settings.acpAgents.fields.commandBuiltinDescription', { command: activeBuiltin.defaultCommand })
                    : t('settings.acpAgents.fields.commandDescription')
                "
              >
                <Input
                  v-model="draft.command"
                  class="font-mono"
                  :placeholder="activeBuiltin?.defaultCommand || t('settings.acpAgents.commandPlaceholder')"
                  :aria-label="t('settings.acpAgents.fields.command')"
                  :disabled="saving"
                />
              </SettingsRow>

              <SettingsRow
                :title="t('settings.acpAgents.fields.args')"
                :description="
                  activeBuiltin && activeBuiltin.defaultArgs.length
                    ? t('settings.acpAgents.fields.argsBuiltinDescription', { args: activeBuiltin.defaultArgs.join(' ') })
                    : t('settings.acpAgents.fields.argsDescription')
                "
              >
                <textarea
                  v-model="draft.argsText"
                  rows="3"
                  class="w-full rounded-md border border-input bg-background px-2.5 py-2 font-mono text-xs text-foreground outline-none focus:border-ring disabled:opacity-60"
                  :placeholder="t('settings.acpAgents.argsPlaceholder')"
                  :aria-label="t('settings.acpAgents.fields.args')"
                  :disabled="saving"
                />
              </SettingsRow>

              <div
                v-if="effectivePreview"
                class="rounded-md border border-border/60 bg-muted/30 px-3 py-2 text-xs"
              >
                <p class="text-[11px] font-medium text-muted-foreground">{{ t('settings.acpAgents.launchPreview') }}</p>
                <p class="mt-1 break-all font-mono text-foreground">{{ effectivePreview }}</p>
              </div>
            </div>
          </SettingsCard>

          <SettingsCard
            :title="t('settings.acpAgents.fields.env')"
            :description="t('settings.acpAgents.fields.envDescription')"
            :count="draft.envRows.length || undefined"
            heading="h3"
          >
            <template #actions>
              <Button size="sm" variant="outline" :disabled="saving" @click="addEnvRow">
                <RiAddLine class="mr-1 h-3.5 w-3.5" />
                {{ t('settings.acpAgents.addEnv') }}
              </Button>
            </template>

            <div v-if="draft.envRows.length" class="space-y-2">
              <div
                v-for="(envRow, index) in draft.envRows"
                :key="index"
                class="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] items-center gap-2"
              >
                <Input
                  v-model="envRow.key"
                  class="font-mono text-xs"
                  :placeholder="t('settings.acpAgents.envKeyPlaceholder')"
                  :aria-label="t('settings.acpAgents.envKeyLabel')"
                  :disabled="saving"
                />
                <Input
                  v-model="envRow.value"
                  class="font-mono text-xs"
                  :placeholder="t('settings.acpAgents.envValuePlaceholder')"
                  :aria-label="t('settings.acpAgents.envValueLabel')"
                  :disabled="saving"
                />
                <IconButton
                  size="sm"
                  variant="ghost"
                  class="text-muted-foreground hover:text-destructive"
                  :tooltip="t('common.delete')"
                  :aria-label="t('common.delete')"
                  :disabled="saving"
                  @click="removeEnvRow(index)"
                >
                  <RiDeleteBinLine class="h-4 w-4" />
                </IconButton>
              </div>
            </div>
            <p v-else class="text-xs text-muted-foreground">{{ t('settings.acpAgents.noEnv') }}</p>
          </SettingsCard>

          <SettingsCard
            :title="t('settings.acpAgents.fields.writablePaths')"
            :description="t('settings.acpAgents.fields.writablePathsDescription')"
            :icon="RiShieldCheckLine"
            heading="h3"
          >
            <div class="space-y-3">
              <SettingsRow
                :title="t('settings.acpAgents.fields.clientTools')"
                :description="t('settings.acpAgents.fields.clientToolsDescription')"
                toggle
              >
                <SettingsToggle
                  v-model="draft.clientTools"
                  :label="t('settings.acpAgents.fields.clientTools')"
                  :disabled="saving"
                />
              </SettingsRow>

              <SettingsRow
                :title="t('settings.acpAgents.fields.writablePaths')"
                :description="t('settings.acpAgents.fields.writablePathsDescription')"
              >
                <textarea
                  v-model="draft.writablePathsText"
                  rows="3"
                  class="w-full rounded-md border border-input bg-background px-2.5 py-2 font-mono text-xs text-foreground outline-none focus:border-ring disabled:opacity-60"
                  :placeholder="t('settings.acpAgents.writablePathsPlaceholder')"
                  :aria-label="t('settings.acpAgents.fields.writablePaths')"
                  :disabled="saving"
                />
              </SettingsRow>
            </div>
          </SettingsCard>

          <div
            v-if="validationErrors.length"
            role="alert"
            class="space-y-1 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2.5 text-xs text-destructive"
          >
            <p v-for="(issue, idx) in validationErrors" :key="idx">{{ validationMessage(issue) }}</p>
          </div>

          <SettingsSaveBar
            :dirty="dirty || row.isNew"
            :saving="saving"
            :save-disabled="validationErrors.length > 0"
            :error="saveError"
            @save="saveDraft"
            @discard="discardDraft"
          />
        </fieldset>
      </SettingsDisclosureRow>

      <p
        v-if="!rows.length"
        class="rounded-lg border border-dashed border-border/70 bg-muted/15 px-4 py-8 text-center text-xs text-muted-foreground"
      >
        {{ t('settings.acpAgents.emptySearch') }}
      </p>
    </div>
  </section>
</template>
