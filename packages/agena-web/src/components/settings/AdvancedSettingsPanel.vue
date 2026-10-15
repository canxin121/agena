<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiDeleteBinLine, RiFileCopyLine, RiRefreshLine, RiCodeSSlashLine, RiShieldCheckLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { confirmAction } from '@/lib/appConfirm'
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
const targetLayer = computed<RuntimeSettingsLayer>(() => (props.scope === 'workspace' ? 'workspace' : 'global'))
const settingPath = ref('')
const loadedPath = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const draftText = ref('null')
const savedText = ref('null')
const dirty = computed(() => draftText.value !== savedText.value)
const loading = ref(false)
const activeAction = ref<'validate' | 'save' | 'clear' | ''>('')
const error = ref('')
const parseError = ref('')
const lastAction = ref<JsonValue>(null)

const actionBusy = computed(() => Boolean(activeAction.value))
const busy = computed(() => loading.value || actionBusy.value)
const normalizedPath = computed(() => settingPath.value.trim())
const selectedLayerResponse = computed<RuntimeSettingReadResponse | null>(() => {
  const bundle = sources.value
  if (!bundle) return null
  return targetLayer.value === 'workspace' ? bundle.workspace : bundle.global
})
const loadedForCurrentPath = computed(() => Boolean(loadedPath.value && loadedPath.value === normalizedPath.value))
const selectedLayerLabel = computed(() => (targetLayer.value === 'workspace' ? st('Workspace') : st('Global')))
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
    parseError.value = ''
    return value
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    parseError.value = message
    throw new Error(message)
  }
}

function loadSelectedLayerIntoDraft() {
  draftText.value = jsonText(selectedLayerResponse.value?.value)
  savedText.value = draftText.value
  parseError.value = ''
}

function copyEffectiveIntoDraft() {
  draftText.value = jsonText(sources.value?.effective.value)
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
  if (!path || busy.value || !loadedForCurrentPath.value) return
  activeAction.value = 'validate'
  error.value = ''
  lastAction.value = null
  try {
    const value = parseDraft()
    lastAction.value = await setRuntimeSetting(
      path,
      value,
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
  if (!path || busy.value || !loadedForCurrentPath.value) return
  activeAction.value = 'save'
  error.value = ''
  lastAction.value = null
  try {
    const value = parseDraft()
    lastAction.value = await setRuntimeSetting(path, value, { validate: true, reload: true }, targetLayer.value)
    await load()
    toasts.push('success', st('{layer} setting saved: {path}', { layer: selectedLayerLabel.value, path }))
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    activeAction.value = ''
  }
}

async function clearOverride() {
  const path = normalizedPath.value
  if (!path || busy.value || !loadedForCurrentPath.value) return
  if (!(await confirmAction(st('Delete {layer} override {path}?', { layer: selectedLayerLabel.value, path })))) return
  activeAction.value = 'clear'
  error.value = ''
  lastAction.value = null
  try {
    lastAction.value = await deleteRuntimeSetting(path, { validate: true, reload: true }, targetLayer.value)
    await load()
    toasts.push('success', st('{layer} override cleared: {path}', { layer: selectedLayerLabel.value, path }))
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    activeAction.value = ''
  }
}

async function canLeave() {
  if (busy.value) return false
  if (!dirty.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
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
onBeforeRouteUpdate((to, from) => {
  if (to.path === from.path && to.query.view === from.query.view && to.query.scope === from.query.scope) return true
  return canLeave()
})
onBeforeRouteLeave(canLeave)

watch(targetLayer, () => {
  if (sources.value && loadedForCurrentPath.value) loadSelectedLayerIntoDraft()
})

onMounted(() => {
  settingPath.value = COMMON_PATHS.value[0]?.value || 'providers'
  void load()
})
</script>

<template>
  <div class="min-w-0 space-y-4">
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
      <div class="flex min-w-0 flex-col gap-2 sm:flex-row">
        <OptionPicker
          class="min-w-0 flex-1"
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
      class="grid min-w-0 items-start gap-4 xl:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]"
    >
      <SettingsCard
        :title="$st('{layer} draft', { layer: selectedLayerLabel })"
        :description="$st('File: {path} · {status}', { path: selectedLayerFilePath, status: selectedLayerFileStatus })"
        class="[&_header_p]:break-all"
      >
        <template #actions>
          <Button variant="ghost" size="sm" :disabled="busy" @click="copyEffectiveIntoDraft"
            ><RiFileCopyLine class="mr-1.5 h-4 w-4 shrink-0" />{{ $st('Copy effective') }}</Button
          >
        </template>
        <label class="mb-2 block break-all font-mono text-xs text-muted-foreground" for="advanced-setting-draft">{{
          normalizedPath || $st('No path selected')
        }}</label>
        <textarea
          dir="ltr"
          id="advanced-setting-draft"
          v-model="draftText"
          :rows="Math.min(20, Math.max(10, draftText.split('\n').length + 1))"
          spellcheck="false"
          :disabled="busy"
          class="w-full min-w-0 resize-y rounded-lg border border-input bg-background/60 p-3 font-mono text-xs leading-6 outline-none focus:border-ring"
        />
        <p v-if="parseError" role="alert" class="mt-2 break-words text-xs text-destructive">{{ parseError }}</p>
        <div class="mt-3 flex flex-wrap items-center justify-between gap-2">
          <Button
            variant="ghost"
            size="sm"
            class="h-auto whitespace-normal text-left text-destructive"
            :disabled="busy"
            @click="clearOverride"
            ><RiDeleteBinLine class="mr-1.5 h-4 w-4 shrink-0" />{{
              $st('Clear {layer} override', { layer: selectedLayerLabel })
            }}</Button
          >
          <Button variant="outline" size="sm" :disabled="busy" @click="validate"
            ><RiShieldCheckLine class="mr-1.5 h-4 w-4 shrink-0" />{{
              activeAction === 'validate' ? $st('Checking…') : $st('Dry-run validate')
            }}</Button
          >
        </div>
      </SettingsCard>
      <SettingsCard :title="$st('Configuration sources')">
        <div class="space-y-3">
          <details class="min-w-0 rounded-lg border border-border/60 bg-background/30" open>
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
      :save-disabled="!loadedForCurrentPath"
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
