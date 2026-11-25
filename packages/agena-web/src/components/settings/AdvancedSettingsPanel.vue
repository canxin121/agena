<script setup lang="ts">
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import { computed, onMounted, ref, watch } from 'vue'
import { RiRefreshLine, RiCodeSSlashLine, RiShieldCheckLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsSourceControl from './SettingsSourceControl.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { confirmAction } from '@/lib/appConfirm'
import { hasRuntimeLayerValue, runtimeSettingBaseValue } from '@/lib/runtimeSettingLayer'
import {
  deleteRuntimeSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingReadResponse,
  type RuntimeSettingsLayer,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import { parsePermissionConfig, permissionConfigErrorMessage } from '@/lib/permissionConfig'

const COMMON_PATHS = computed(() => [
  {
    value: 'providers',
    label: 'providers',
    description: st('Provider configuration, authentication, and model routes.'),
  },
  {
    value: 'permission',
    label: 'permission',
    description: st('Global or workspace permission policy, including the automatic approval model.'),
  },
  {
    value: 'plugins',
    label: 'plugins',
    description: st('Plugin host policy and configured plugin records.'),
  },
  {
    value: 'runtime.providers.client_versions',
    label: 'runtime.providers.client_versions',
    description: st('Provider identity compatibility versions.'),
  },
  {
    value: 'session.compaction',
    label: 'session.compaction',
    description: st('Session compaction defaults.'),
  },
  {
    value: 'ui',
    label: 'ui',
    description: st('Server-backed UI and TUI preferences.'),
  },
  {
    value: 'tracing',
    label: 'tracing',
    description: st('Tracing filters and diagnostic output policy.'),
  },
])

const toasts = useToastsStore()
// The write target is chosen in the workbench and named in the section
// header and the draft below, so this editor only follows the forwarded layer.
const props = defineProps<{ scope: string }>()
const editable = computed(() => props.scope === 'global' || props.scope === 'workspace')
const targetLayer = computed<RuntimeSettingsLayer>(() => (props.scope === 'workspace' ? 'workspace' : 'global'))
const settingPath = ref('')
const loadedPath = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const draftText = ref('null')
const savedText = ref('null')
const sourceMode = ref<'inherit' | 'local'>('inherit')
const savedSourceMode = ref<'inherit' | 'local'>('inherit')
const localDraftReady = ref(false)
let suspendedLocalDraft: { text: string; ready: boolean } | null = null
const confirmingSource = ref(false)
const confirmedOverridePath = ref('')
const dirty = computed(
  () =>
    sourceMode.value !== savedSourceMode.value || (sourceMode.value === 'local' && draftText.value !== savedText.value),
)
const loading = ref(false)
const activeAction = ref<'validate' | 'save' | ''>('')
const error = ref('')
const parseError = ref('')
const lastAction = ref<JsonValue>(null)

const actionBusy = computed(() => Boolean(activeAction.value))
const busy = computed(() => loading.value || actionBusy.value || confirmingSource.value)
const normalizedPath = computed(() => settingPath.value.trim())
const selectedLayerResponse = computed<RuntimeSettingReadResponse | null>(() => {
  const bundle = sources.value
  if (!bundle) return null
  if (!editable.value) return bundle.effective
  return targetLayer.value === 'workspace' ? bundle.workspace : bundle.global
})
const loadedForCurrentPath = computed(() => Boolean(loadedPath.value && loadedPath.value === normalizedPath.value))
const selectedLayerLabel = computed(() =>
  !editable.value ? st('Effective') : targetLayer.value === 'workspace' ? st('Workspace') : st('Global'),
)
const inheritedValue = computed(() => runtimeSettingBaseValue(selectedLayerResponse.value, targetLayer.value))
const inheritedKnown = computed(() => inheritedValue.value !== undefined)
const sourceValueHeading = computed(() =>
  targetLayer.value === 'global' ? st('Default value') : st('Inherited value'),
)
const groupOverridePath = computed(() => {
  const response = selectedLayerResponse.value
  const path = response?.override_path
  return hasRuntimeLayerValue(response) && path && path !== normalizedPath.value ? path : ''
})
const requiresGroupReset = computed(
  () => Boolean(groupOverridePath.value) && selectedLayerResponse.value?.requires_group_reset !== false,
)
const inheritedReadOnly = computed(() => targetLayer.value !== 'global' && sourceMode.value === 'inherit')
const valueDisabled = computed(
  () => !editable.value || busy.value || requiresGroupReset.value || inheritedReadOnly.value,
)
const displayedDraftText = computed(() =>
  inheritedReadOnly.value
    ? inheritedKnown.value
      ? jsonText(inheritedValue.value)
      : st('Unavailable')
    : draftText.value,
)
const mutationBlocked = computed(
  () =>
    !editable.value ||
    (sourceMode.value === 'local' && !localDraftReady.value) ||
    (Boolean(groupOverridePath.value) &&
      (sourceMode.value === 'inherit'
        ? confirmedOverridePath.value !== groupOverridePath.value
        : requiresGroupReset.value)),
)
const selectedLayerFilePath = computed(() => selectedLayerResponse.value?.config_path || st('not reported'))
const selectedLayerFileStatus = computed(() =>
  selectedLayerResponse.value?.config_found ? st('file exists') : st('created on first write'),
)

function jsonText(value: JsonValue): string {
  if (value === undefined) return 'null'
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function parseDraft(): JsonValue {
  try {
    const value = JSON.parse(draftText.value) as JsonValue
    const validated = normalizedPath.value === 'permission' ? (parsePermissionConfig(value) as JsonValue) : value
    parseError.value = ''
    return validated
  } catch (reason) {
    const message = permissionConfigErrorMessage(reason, st)
    parseError.value = message
    throw new Error(message)
  }
}

function loadSelectedLayerIntoDraft() {
  sourceMode.value = hasRuntimeLayerValue(selectedLayerResponse.value) ? 'local' : 'inherit'
  savedSourceMode.value = sourceMode.value
  // This is the raw document editor. Resolved/default objects are reference
  // data and must not become saved overrides when only one JSON field changes.
  draftText.value = jsonText(selectedLayerResponse.value?.value)
  savedText.value = draftText.value
  localDraftReady.value = sourceMode.value === 'local'
  suspendedLocalDraft = null
  confirmedOverridePath.value = ''
  parseError.value = ''
}

async function chooseSource(value: 'inherit' | 'local') {
  if (!editable.value) return
  if (targetLayer.value === 'global' && value === 'local') return
  if (busy.value || !loadedForCurrentPath.value || sourceMode.value === value) return
  if (value === 'local' && groupOverridePath.value) {
    const draft = suspendedLocalDraft
    loadSelectedLayerIntoDraft()
    if (draft) {
      draftText.value = draft.text
      localDraftReady.value = draft.ready
    }
    return
  }
  if (value === 'inherit' && groupOverridePath.value) {
    const target = targetLayer.value
    const scope = props.scope
    const leaf = normalizedPath.value
    const path = groupOverridePath.value
    confirmingSource.value = true
    let accepted = false
    try {
      accepted = await confirmAction({
        title:
          target === 'global'
            ? st('Restore defaults for {path}?', { path })
            : st('Restore inheritance for {path}?', { path }),
        description:
          target === 'global'
            ? st('This also restores the default values of the other settings in this group.')
            : st('This also restores the other settings in this group.'),
        confirmText: target === 'global' ? st('Reset to defaults') : st('Use inherited value'),
        cancelText: st('Cancel'),
        variant: 'default',
      })
    } finally {
      confirmingSource.value = false
    }
    if (
      !accepted ||
      scope !== props.scope ||
      target !== targetLayer.value ||
      leaf !== normalizedPath.value ||
      path !== groupOverridePath.value
    )
      return
    confirmedOverridePath.value = path
  }
  if (value === 'inherit') {
    suspendedLocalDraft = { text: draftText.value, ready: localDraftReady.value }
    draftText.value = 'null'
    localDraftReady.value = false
  } else if (suspendedLocalDraft) {
    draftText.value = suspendedLocalDraft.text
    localDraftReady.value = suspendedLocalDraft.ready
    suspendedLocalDraft = null
  } else {
    localDraftReady.value = inheritedKnown.value && inheritedValue.value !== null
    draftText.value = localDraftReady.value ? jsonText(inheritedValue.value) : ''
  }
  sourceMode.value = value
  parseError.value = ''
}
function editDraft(event: Event) {
  if (valueDisabled.value) return
  sourceMode.value = 'local'
  draftText.value = (event.target as HTMLTextAreaElement).value
  localDraftReady.value = true
  parseError.value = ''
}

async function load(options: { preserveDraft?: boolean } = {}) {
  const path = normalizedPath.value
  if (!path || loading.value) return
  loading.value = true
  error.value = ''
  try {
    sources.value = await readRuntimeSettingSources(path)
    loadedPath.value = path
    if (!options.preserveDraft) loadSelectedLayerIntoDraft()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    // Keep the last successful read and the draft available after a failed refresh.
  } finally {
    loading.value = false
  }
}

async function validate() {
  const path = normalizedPath.value
  if (!path || busy.value || !loadedForCurrentPath.value || mutationBlocked.value) return
  const editPath = groupOverridePath.value || path
  activeAction.value = 'validate'
  error.value = ''
  lastAction.value = null
  try {
    lastAction.value =
      sourceMode.value === 'inherit'
        ? await deleteRuntimeSetting(editPath, { dry_run: true, validate: true, reload: false }, targetLayer.value)
        : await setRuntimeSetting(
            path,
            parseDraft(),
            { dry_run: true, validate: true, reload: false },
            targetLayer.value,
          )
    toasts.push('success', st('{layer} setting is valid: {path}', { layer: selectedLayerLabel.value, path }))
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    activeAction.value = ''
  }
}

async function save() {
  const path = normalizedPath.value
  if (!path || busy.value || !loadedForCurrentPath.value || !dirty.value || mutationBlocked.value) return
  const editPath = groupOverridePath.value || path
  activeAction.value = 'save'
  error.value = ''
  lastAction.value = null
  try {
    lastAction.value =
      sourceMode.value === 'inherit'
        ? await deleteRuntimeSetting(editPath, { validate: true, reload: true }, targetLayer.value)
        : await setRuntimeSetting(path, parseDraft(), { validate: true, reload: true }, targetLayer.value)
    await load()
    toasts.push('success', st('{layer} setting saved: {path}', { layer: selectedLayerLabel.value, path }))
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    activeAction.value = ''
  }
}

const { canDiscard: canLeave } = useSettingsDraftGuard({
  dirty: () => dirty.value,
  busy: () => busy.value,
  unload: false,
  sameTarget: (to, from) => to.path === from.path && to.query.view === from.query.view && to.query.scope === from.query.scope,
})
async function choosePath(value: string) {
  if (value === settingPath.value || !(await canLeave())) return
  settingPath.value = value
  lastAction.value = null
  await load()
}
async function refresh() {
  await load({ preserveDraft: dirty.value })
}
defineExpose({ refresh })

watch(
  () => props.scope,
  () => {
    if (sources.value && loadedForCurrentPath.value) loadSelectedLayerIntoDraft()
  },
)

onMounted(() => {
  settingPath.value = COMMON_PATHS.value[0]?.value || 'providers'
  void load()
})
</script>

<template>
  <div class="settings-stack">
    <SettingsCard :title="$st('Configuration path')" :icon="RiCodeSSlashLine">
      <template #actions>
        <IconButton
          variant="outline"
          size="sm"
          :disabled="busy || !normalizedPath"
          :tooltip="$st('Reload setting sources')"
          :aria-label="$st('Reload advanced setting sources')"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </template>
      <div class="settings-toolbar">
        <OptionPicker
          class="settings-toolbar-search"
          :model-value="settingPath"
          :options="COMMON_PATHS"
          :title="$st('Configuration path')"
          :placeholder="$st('Choose or type a JSON path')"
          :search-placeholder="$st('Search common paths or type a custom path')"
          :include-empty="false"
          :allow-custom="true"
          monospace
          :disabled="busy"
          @update:model-value="choosePath"
        />
        <Button variant="outline" :disabled="busy || !normalizedPath" @click="refresh">{{ $st('Load path') }}</Button>
      </div>
      <p class="mt-3 text-xs leading-relaxed text-muted-foreground">
        {{
          $st(
            'This editor can expose and change credentials or security policy. Prefer the dedicated Provider, Permission, Plugin, and MCP pages when one exists.',
          )
        }}
      </p>
    </SettingsCard>
    <p
      v-if="error"
      role="alert"
      class="break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <div
      v-if="sources && loadedForCurrentPath"
      class="settings-columns"
    >
      <SettingsCard
        :title="$st('{layer} draft', { layer: selectedLayerLabel })"
        :description="$st('File: {path} · {status}', { path: selectedLayerFilePath, status: selectedLayerFileStatus })"
        class="[&_header_p]:break-all"
        data-advanced-setting
      >
        <template #actions>
          <SettingsSourceControl
            :model-value="editable ? sourceMode : 'effective'"
            :scope="scope"
            :label="normalizedPath"
            :inherited-source="targetLayer === 'workspace' ? $st('Global layer') : $st('Built-in default')"
            :inherited-value="inheritedKnown ? jsonText(inheritedValue) : $st('Unavailable')"
            :disabled="busy"
            @update:model-value="chooseSource"
          />
        </template>
        <label class="mb-2 block break-all font-mono text-xs text-muted-foreground" for="advanced-setting-draft">{{
          normalizedPath || $st('No path selected')
        }}</label>
        <p v-if="groupOverridePath" class="mb-2 break-all text-xs text-muted-foreground" data-setting-group-override>
          {{ $st('Group override: {path}', { path: groupOverridePath }) }}
        </p>
        <textarea
          dir="ltr"
          id="advanced-setting-draft"
          :value="displayedDraftText"
          @input="editDraft"
          :rows="Math.min(20, Math.max(10, displayedDraftText.split('\n').length + 1))"
          spellcheck="false"
          :disabled="valueDisabled"
          class="w-full min-w-0 resize-y rounded-md border border-input bg-background/60 p-2.5 font-mono text-xs leading-5 outline-none focus:border-ring disabled:cursor-not-allowed disabled:opacity-60"
        />
        <p v-if="parseError" role="alert" class="mt-2 break-words text-xs text-destructive">{{ parseError }}</p>
        <div class="mt-3 flex flex-wrap items-center justify-between gap-2">
          <span class="text-xs text-muted-foreground">{{
            targetLayer === 'global'
              ? sourceMode === 'inherit' && dirty
                ? $st('Save to restore default settings.')
                : ''
              : sourceMode === 'inherit'
                ? dirty
                  ? $st('Save to remove this scope’s override.')
                  : sourceValueHeading
                : $st('This scope')
          }}</span>
          <Button
            variant="outline"
            size="sm"
            :disabled="busy || !loadedForCurrentPath || mutationBlocked"
            @click="validate"
            ><RiShieldCheckLine class="mr-1.5 h-4 w-4 shrink-0" />{{
              activeAction === 'validate' ? $st('Checking…') : $st('Dry-run validate')
            }}</Button
          >
        </div>
      </SettingsCard>
      <SettingsCard :title="$st('Configuration sources')">
        <div class="settings-stack">
          <details v-if="editable" class="min-w-0 rounded-lg border border-border/60 bg-background/30" open>
            <summary class="cursor-pointer px-3 py-2.5 text-sm font-medium">{{ sourceValueHeading }}</summary>
            <pre dir="ltr" class="max-h-80 overflow-auto border-t border-border/50 p-3 font-mono text-xs leading-5">{{
              inheritedKnown ? jsonText(inheritedValue) : $st('Unavailable')
            }}</pre>
          </details>
          <details class="min-w-0 rounded-lg border border-border/60 bg-background/30">
            <summary class="cursor-pointer px-3 py-2.5 text-sm font-medium">{{ $st('Effective value') }}</summary>
            <pre dir="ltr" class="max-h-80 overflow-auto border-t border-border/50 p-3 font-mono text-xs leading-5">{{
              jsonText(sources.effective.value)
            }}</pre>
          </details>
          <details class="min-w-0 rounded-lg border border-border/60 bg-background/30">
            <summary class="cursor-pointer px-3 py-2.5 text-sm font-medium">{{ $st('Global layer') }}</summary>
            <pre dir="ltr" class="max-h-64 overflow-auto border-t border-border/50 p-3 font-mono text-xs leading-5">{{
              jsonText(sources.global.value)
            }}</pre>
          </details>
          <details class="min-w-0 rounded-lg border border-border/60 bg-background/30">
            <summary class="cursor-pointer px-3 py-2.5 text-sm font-medium">{{ $st('Workspace layer') }}</summary>
            <pre dir="ltr" class="max-h-64 overflow-auto border-t border-border/50 p-3 font-mono text-xs leading-5">{{
              jsonText(sources.workspace.value)
            }}</pre>
          </details>
        </div>
      </SettingsCard>
    </div>
    <SettingsSaveBar
      v-if="dirty || activeAction === 'save'"
      :dirty="dirty"
      :saving="activeAction === 'save'"
      :disabled="busy"
      :save-disabled="!loadedForCurrentPath || mutationBlocked"
      :save-label="$st('Save & reload')"
      sticky
      @save="save"
      @discard="loadSelectedLayerIntoDraft"
    />
    <details v-if="lastAction" class="min-w-0 rounded-xl border border-border/60 bg-card/30">
      <summary class="cursor-pointer px-4 py-3 text-sm font-medium">{{ $st('Last edit response') }}</summary>
      <pre dir="ltr" class="max-h-80 overflow-auto border-t border-border/60 p-3 font-mono text-xs leading-5">{{
        jsonText(lastAction)
      }}</pre>
    </details>
  </div>
</template>
