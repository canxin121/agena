<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RiDeleteBinLine, RiFileCopyLine, RiRefreshLine, RiSave3Line, RiShieldCheckLine } from '@remixicon/vue'

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

const COMMON_PATHS = [
  {
    value: 'providers',
    label: 'providers',
    description: st('Provider inventory, adapters, authentication, and model routes.'),
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
  {
    value: 'harnesses',
    label: 'harnesses',
    description: st('Browser, shell, and editor harness catalogs.'),
  },
]

const layerOptions = [
  { value: 'global', label: st('Global layer'), description: st('Writes the server-wide Agena configuration file.') },
  {
    value: 'workspace',
    label: st('Workspace layer'),
    description: st('Writes the current workspace configuration file.'),
  },
]

const toasts = useToastsStore()
const targetLayer = ref<RuntimeSettingsLayer>('global')
const settingPath = ref('')
const loadedPath = ref('')
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const draftText = ref('null')
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
    sources.value = null
    loadedPath.value = ''
  } finally {
    loading.value = false
  }
}

async function validate() {
  const path = normalizedPath.value
  if (!path || actionBusy.value) return
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
  if (!path || actionBusy.value) return
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
  if (!path || actionBusy.value) return
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

function choosePath(value: string) {
  settingPath.value = value
  void load()
}

watch(targetLayer, () => {
  if (sources.value && loadedForCurrentPath.value) loadSelectedLayerIntoDraft()
})

onMounted(() => {
  settingPath.value = COMMON_PATHS[0]?.value || 'providers'
  void load()
})
</script>

<template>
  <div class="grid min-w-0 gap-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">{{ $st('Advanced configuration path editor') }}</h2>
        <p class="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">
          {{
            $st(
              'Edit any server configuration path that does not yet have a dedicated form. Writes always target an explicit Global or Workspace layer, run full composed-config validation, and request a runtime reload.',
            )
          }}
        </p>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :disabled="busy || !normalizedPath"
        :tooltip="loading ? $st('Loading setting sources') : $st('Reload setting sources')"
        :aria-label="$st('Reload advanced setting sources')"
        @click="load()"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div
      class="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-800 dark:text-amber-200"
    >
      {{
        $st(
          'This editor can expose and change credentials or security policy. Prefer the dedicated Provider, Permission, Plugin, MCP, and Harness pages when one exists.',
        )
      }}
    </div>

    <section class="grid gap-3 rounded-lg border border-border/60 bg-muted/10 p-4">
      <div class="grid gap-3 lg:grid-cols-[13rem_minmax(0,1fr)_auto] lg:items-end">
        <label class="grid gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('Write target') }}</span>
          <OptionPicker
            v-model="targetLayer"
            :options="layerOptions"
            :include-empty="false"
            :title="$st('Configuration layer')"
            :disabled="busy"
          />
        </label>
        <label class="grid min-w-0 gap-1.5">
          <span class="text-xs text-muted-foreground">{{ $st('JSON path') }}</span>
          <OptionPicker
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
        </label>
        <Button variant="outline" :disabled="busy || !normalizedPath" @click="load()">{{ $st('Load path') }}</Button>
      </div>
      <code class="break-all font-mono text-[11px] text-muted-foreground">
        {{ normalizedPath || $st('No path selected') }}
      </code>
    </section>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <section v-if="sources" class="grid min-w-0 gap-4">
      <div class="grid gap-3 lg:grid-cols-3">
        <details class="min-w-0 rounded-lg border border-border/60" open>
          <summary class="cursor-pointer px-3 py-2 text-sm font-medium">{{ $st('Effective value') }}</summary>
          <pre class="max-h-60 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
            jsonText(sources.effective.value)
          }}</pre>
        </details>
        <details class="min-w-0 rounded-lg border border-border/60">
          <summary class="cursor-pointer px-3 py-2 text-sm font-medium">{{ $st('Global layer') }}</summary>
          <pre class="max-h-60 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
            jsonText(sources.global.value)
          }}</pre>
        </details>
        <details class="min-w-0 rounded-lg border border-border/60">
          <summary class="cursor-pointer px-3 py-2 text-sm font-medium">{{ $st('Workspace layer') }}</summary>
          <pre class="max-h-60 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
            jsonText(sources.workspace.value)
          }}</pre>
        </details>
      </div>

      <section class="grid gap-3 rounded-lg border border-border/60 p-4">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 class="text-sm font-semibold">{{ $st('{layer} draft', { layer: selectedLayerLabel }) }}</h3>
            <p class="mt-1 text-xs text-muted-foreground">
              {{ $st('File: {path} · {status}', { path: selectedLayerFilePath, status: selectedLayerFileStatus }) }}
            </p>
          </div>
          <div class="flex flex-wrap gap-2">
            <Button variant="ghost" size="sm" :disabled="busy" @click="loadSelectedLayerIntoDraft">
              <RiRefreshLine class="mr-1.5 h-4 w-4" /> {{ $st('Revert draft') }}
            </Button>
            <Button variant="ghost" size="sm" :disabled="busy" @click="copyEffectiveIntoDraft">
              <RiFileCopyLine class="mr-1.5 h-4 w-4" /> {{ $st('Copy effective') }}
            </Button>
          </div>
        </div>
        <textarea
          v-model="draftText"
          rows="20"
          spellcheck="false"
          :disabled="busy"
          class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs leading-5 outline-none focus:border-ring"
        />
        <div v-if="parseError" class="text-xs text-destructive">{{ parseError }}</div>
        <div class="flex flex-wrap items-center justify-between gap-3 border-t border-border/60 pt-3">
          <Button variant="ghost" size="sm" class="text-destructive" :disabled="busy" @click="clearOverride">
            <RiDeleteBinLine class="mr-1.5 h-4 w-4" />
            {{ $st('Clear {layer} override', { layer: selectedLayerLabel }) }}
          </Button>
          <div class="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" :disabled="busy" @click="validate">
              <RiShieldCheckLine class="mr-1.5 h-4 w-4" />
              {{ activeAction === 'validate' ? $st('Checking…') : $st('Dry-run validate') }}
            </Button>
            <Button size="sm" :disabled="busy" @click="save">
              <RiSave3Line class="mr-1.5 h-4 w-4" />
              {{ activeAction === 'save' ? $st('Saving…') : $st('Save & reload') }}
            </Button>
          </div>
        </div>
      </section>

      <details v-if="lastAction" class="rounded-lg border border-border/60">
        <summary class="cursor-pointer px-4 py-3 text-sm font-medium">{{ $st('Last edit response') }}</summary>
        <pre class="max-h-80 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
          jsonText(lastAction)
        }}</pre>
      </details>
    </section>
  </div>
</template>
