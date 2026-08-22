<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiAddLine, RiDeleteBinLine, RiFileCopyLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  deleteRuntimeSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingsLayer,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import type { JsonObject, JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

type BrowserHarness = {
  driver: string
  headless: boolean
  viewport?: { width: number; height: number }
  allowed_domains?: string[]
  launch_options?: JsonValue
}
type ShellHarness = {
  workspace_only: boolean
  allow_commands?: string[]
  deny_commands?: string[]
  env?: Record<string, string>
}
type EditorHarness = { workspace_only: boolean; max_file_bytes?: number | null; allowed_extensions?: string[] }
type HarnessKind = 'browser' | 'shell' | 'editor'
type HarnessConfig = BrowserHarness | ShellHarness | EditorHarness
type HarnessMap = Record<string, HarnessConfig>

const selectedKind = ref<HarnessKind>('browser')
const targetLayer = ref<RuntimeSettingsLayer>('global')
const maps = ref<Record<HarnessKind, HarnessMap>>({ browser: {}, shell: {}, editor: {} })
const sources = ref<Partial<Record<HarnessKind, RuntimeSettingsReadBundle>>>({})
const selectedName = ref('')
const newName = ref('')
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const jsonError = ref('')
const rawHarnessJson = ref('{}')
const rawHarnessJsonDirty = ref(false)

const kindOptions = [
  {
    value: 'browser',
    label: st('Browser Harness'),
    description: st('Browser driver, domains, viewport, and launch options.'),
  },
  {
    value: 'shell',
    label: st('Shell Harness'),
    description: st('Workspace boundary, command allow/deny lists, and environment.'),
  },
  {
    value: 'editor',
    label: st('Editor Harness'),
    description: st('Workspace boundary, file size, and extension allowlist.'),
  },
]
const layerOptions = [
  { value: 'global', label: st('Global layer'), description: st('Available to all workspaces.') },
  { value: 'workspace', label: st('Workspace layer'), description: st('Overrides only the current workspace.') },
]

const names = computed(() => Object.keys(maps.value[selectedKind.value] || {}).sort((a, b) => a.localeCompare(b)))
const selectedConfig = computed(() =>
  selectedName.value ? maps.value[selectedKind.value]?.[selectedName.value] : null,
)
const browserConfig = computed(() => selectedConfig.value as BrowserHarness | null)
const shellConfig = computed(() => selectedConfig.value as ShellHarness | null)
const editorConfig = computed(() => selectedConfig.value as EditorHarness | null)
const selectedSource = computed(() => sources.value[selectedKind.value])
const selectedLayerResponse = computed(() =>
  targetLayer.value === 'workspace' ? selectedSource.value?.workspace : selectedSource.value?.global,
)
const settingPath = computed(() => `harnesses.${selectedKind.value}`)
const targetLayerLabel = computed(() => (targetLayer.value === 'workspace' ? st('Workspace') : st('Global')))
const selectedLayerHarnessCount = computed(() =>
  selectedLayerResponse.value?.value ? Object.keys(asRecord(selectedLayerResponse.value.value)).length : null,
)
const selectedLayerSummary = computed(() =>
  selectedLayerHarnessCount.value === null
    ? st('Editing {layer} layer · unset', { layer: targetLayerLabel.value })
    : st('Editing {layer} layer · {count} harnesses', {
        layer: targetLayerLabel.value,
        count: selectedLayerHarnessCount.value,
      }),
)
const busy = computed(() => loading.value || saving.value)
let rawHarnessJsonSyncTimer: ReturnType<typeof setTimeout> | null = null

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function cloneEditableObject(value: JsonObject): JsonObject {
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => [
      key,
      Array.isArray(item) ? [...item] : item && typeof item === 'object' ? { ...(item as JsonObject) } : item,
    ]),
  ) as JsonObject
}

function asRecord(value: unknown): HarnessMap {
  return value && typeof value === 'object' && !Array.isArray(value) ? clone(value as HarnessMap) : {}
}

function arrayText(value: unknown): string {
  return Array.isArray(value) ? value.map((item) => String(item)).join(', ') : ''
}

function parseList(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)
}

function jsonText(value: JsonValue): string {
  if (value === undefined || value === null) return ''
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function envText(value: Record<string, string> | null | undefined): string {
  return Object.entries(value || {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, item]) => `${key}=${item}`)
    .join('\n')
}

function parseEnv(value: string): Record<string, string> {
  const output: Record<string, string> = {}
  for (const [index, rawLine] of value.split(/\r?\n/).entries()) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#')) continue
    const separator = line.indexOf('=')
    if (separator <= 0) throw new Error(st('Environment line {index} must use KEY=VALUE.', { index: index + 1 }))
    const key = line.slice(0, separator).trim()
    if (!key) throw new Error(st('Environment line {index} has an empty key.', { index: index + 1 }))
    output[key] = line.slice(separator + 1)
  }
  return output
}

function sourceValue(bundle: RuntimeSettingsReadBundle, layer: RuntimeSettingsLayer): JsonValue {
  return layer === 'workspace' ? bundle.workspace.value : bundle.global.value
}

function clearRawHarnessJsonSyncTimer() {
  if (rawHarnessJsonSyncTimer !== null) {
    clearTimeout(rawHarnessJsonSyncTimer)
    rawHarnessJsonSyncTimer = null
  }
}

function syncRawHarnessJson() {
  clearRawHarnessJsonSyncTimer()
  rawHarnessJson.value = selectedConfig.value ? JSON.stringify(selectedConfig.value, null, 2) : '{}'
  rawHarnessJsonDirty.value = false
  jsonError.value = ''
}

function scheduleRawHarnessJsonSync() {
  if (rawHarnessJsonDirty.value) return
  clearRawHarnessJsonSyncTimer()
  rawHarnessJsonSyncTimer = setTimeout(() => {
    rawHarnessJsonSyncTimer = null
    if (!rawHarnessJsonDirty.value) syncRawHarnessJson()
  }, 150)
}

function markRawHarnessJsonDirty() {
  rawHarnessJsonDirty.value = true
  clearRawHarnessJsonSyncTimer()
}

function selectName(name: string) {
  selectedName.value = name
  syncRawHarnessJson()
}

function addHarness() {
  const name = newName.value.trim()
  if (!name) return
  if (Object.prototype.hasOwnProperty.call(maps.value[selectedKind.value], name)) {
    error.value = st('Harness already exists in the {targetLayer} layer: {name}', {
      targetLayer: targetLayer.value,
      name: name,
    })
    return
  }
  const defaults: Record<HarnessKind, HarnessConfig> = {
    browser: {
      driver: 'playwright',
      headless: true,
      viewport: { width: 1280, height: 800 },
      allowed_domains: [],
      launch_options: null,
    },
    shell: { workspace_only: true, allow_commands: [], deny_commands: [], env: {} },
    editor: { workspace_only: true, max_file_bytes: null, allowed_extensions: [] },
  }
  maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [name]: defaults[selectedKind.value] }
  selectedName.value = name
  newName.value = ''
  error.value = ''
  syncRawHarnessJson()
}

function renameHarness(event: Event) {
  const input = event.target as HTMLInputElement
  const currentName = selectedName.value
  const nextName = input.value.trim()
  if (!nextName || nextName === currentName) {
    input.value = currentName
    return
  }
  if (Object.prototype.hasOwnProperty.call(maps.value[selectedKind.value], nextName)) {
    error.value = st('Harness already exists in the {targetLayer} layer: {nextName}', {
      targetLayer: targetLayer.value,
      nextName: nextName,
    })
    input.value = currentName
    return
  }
  const next = { ...maps.value[selectedKind.value] }
  const value = next[currentName]
  if (!value) return
  delete next[currentName]
  next[nextName] = value
  maps.value[selectedKind.value] = next
  selectedName.value = nextName
  error.value = ''
  input.value = nextName
}

function updateSelected(mutator: (value: JsonObject) => void) {
  if (!selectedName.value) return
  // Structured harness edits only replace top-level fields (or the one-level
  // viewport/env objects). Avoid JSON cloning the complete harness, which can
  // include arbitrary launch options, for every character typed into a field.
  const current = cloneEditableObject((selectedConfig.value || {}) as JsonObject)
  mutator(current)
  maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [selectedName.value]: current as HarnessConfig }
  // Keep the raw escape hatch in sync without serializing a potentially large
  // launch_options/env object for every character typed into a structured field.
  scheduleRawHarnessJsonSync()
}

function setBrowserField(key: string, value: string | boolean | number) {
  updateSelected((current) => {
    if (key === 'headless') current.headless = value === true
    else if (key === 'width' || key === 'height') {
      const viewport = current.viewport && typeof current.viewport === 'object' ? current.viewport : {}
      viewport[key] = Number(value) || 0
      current.viewport = viewport
    } else if (key === 'allowed_domains') current.allowed_domains = parseList(String(value))
    else current[key] = value
  })
}

function setBrowserLaunchOptions(value: string) {
  try {
    const trimmed = value.trim()
    const parsed = trimmed ? (JSON.parse(trimmed) as JsonValue) : null
    updateSelected((current) => {
      current.launch_options = parsed
    })
    jsonError.value = ''
  } catch (reason) {
    jsonError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

function setShellField(key: string, value: string | boolean) {
  updateSelected((current) => {
    if (key === 'workspace_only') current.workspace_only = value === true
    else if (key === 'allow_commands' || key === 'deny_commands') current[key] = parseList(String(value))
    else current[key] = value
  })
}

function setShellEnv(value: string) {
  try {
    const env = parseEnv(value)
    updateSelected((current) => {
      current.env = env
    })
    jsonError.value = ''
  } catch (reason) {
    jsonError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

function setEditorField(key: string, value: string | boolean | number) {
  updateSelected((current) => {
    if (key === 'workspace_only') current.workspace_only = value === true
    else if (key === 'allowed_extensions') current.allowed_extensions = parseList(String(value))
    else if (key === 'max_file_bytes') current.max_file_bytes = String(value).trim() ? Number(value) : null
  })
}

function applyJson(value = rawHarnessJson.value) {
  try {
    const parsed = JSON.parse(value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error(st('Harness config must be a JSON object.'))
    }
    if (!selectedName.value) return
    maps.value[selectedKind.value] = {
      ...maps.value[selectedKind.value],
      [selectedName.value]: parsed as HarnessConfig,
    }
    syncRawHarnessJson()
    jsonError.value = ''
  } catch (reason) {
    jsonError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

function copyEffectiveToLayer() {
  const effective = asRecord(selectedSource.value?.effective.value)
  maps.value[selectedKind.value] = effective
  selectedName.value = Object.keys(effective).sort()[0] || ''
  syncRawHarnessJson()
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const next: Partial<Record<HarnessKind, RuntimeSettingsReadBundle>> = {}
    for (const kind of ['browser', 'shell', 'editor'] as HarnessKind[]) {
      const bundle = await readRuntimeSettingSources(`harnesses.${kind}`)
      next[kind] = bundle
      maps.value[kind] = asRecord(sourceValue(bundle, targetLayer.value))
    }
    sources.value = next
    if (!selectedName.value || !names.value.includes(selectedName.value)) selectedName.value = names.value[0] || ''
    syncRawHarnessJson()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function save() {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    await setRuntimeSetting(
      settingPath.value,
      maps.value[selectedKind.value] as JsonValue,
      { reload: true },
      targetLayer.value,
    )
    await load()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}

async function removeHarness() {
  if (
    !selectedName.value ||
    !window.confirm(
      st('Delete {selectedKind} harness {selectedName}?', {
        selectedKind: selectedKind.value,
        selectedName: selectedName.value,
      }),
    )
  )
    return
  const next = { ...maps.value[selectedKind.value] }
  delete next[selectedName.value]
  maps.value[selectedKind.value] = next
  selectedName.value = Object.keys(next).sort()[0] || ''
  await save()
}

async function clearKind() {
  if (
    !window.confirm(
      st('Clear {targetLayer} {settingPath}?', { targetLayer: targetLayer.value, settingPath: settingPath.value }),
    )
  )
    return
  try {
    await deleteRuntimeSetting(settingPath.value, { reload: true }, targetLayer.value)
    await load()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

onMounted(() => void load())
watch(targetLayer, () => void load())
watch(selectedKind, () => {
  if (!names.value.includes(selectedName.value)) selectedName.value = names.value[0] || ''
  syncRawHarnessJson()
})
watch(selectedName, () => syncRawHarnessJson())
onBeforeUnmount(clearRawHarnessJsonSyncTimer)
</script>

<template>
  <section class="grid gap-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">{{ $st('Browser / Shell / Editor Harnesses') }}</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">
          {{
            $st(
              'Edit the selected configuration layer explicitly. Effective values remain visible for comparison and can be copied into the current layer without silently promoting Workspace overrides to Global.',
            )
          }}
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <div class="w-48">
          <OptionPicker
            v-model="targetLayer"
            :options="layerOptions"
            :include-empty="false"
            :title="$st('Harness settings layer')"
            :disabled="busy"
          />
        </div>
        <Button variant="outline" size="sm" :disabled="busy" @click="load">
          <RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" /> {{ $st('Refresh') }}
        </Button>
      </div>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div class="grid gap-4 lg:grid-cols-[minmax(13rem,0.7fr)_minmax(0,2fr)]">
      <div class="grid content-start gap-2">
        <OptionPicker
          v-model="selectedKind"
          :options="kindOptions"
          :include-empty="false"
          :title="$st('Harness kind')"
          :disabled="busy"
        />
        <button
          v-for="name in names"
          :key="name"
          type="button"
          class="rounded-md border px-3 py-2 text-left text-xs"
          :class="selectedName === name ? 'border-primary bg-primary/10' : 'border-border/60 hover:bg-muted/40'"
          :disabled="busy"
          @click="selectName(name)"
        >
          <code>{{ name }}</code>
        </button>
        <div
          v-if="names.length === 0"
          class="rounded-md border border-dashed border-border/60 px-3 py-4 text-center text-xs text-muted-foreground"
        >
          {{ $st('No harnesses configured in the {layer} layer.', { layer: targetLayerLabel }) }}
        </div>
        <div class="flex gap-2">
          <Input
            v-model="newName"
            class="min-w-0 font-mono"
            :placeholder="$st('default')"
            :disabled="busy"
            @keydown.enter="addHarness"
          />
          <IconButton
            variant="outline"
            size="sm"
            :tooltip="$st('Add harness')"
            :aria-label="$st('Add harness')"
            :disabled="busy || !newName.trim()"
            @click="addHarness"
          >
            <RiAddLine class="h-4 w-4" />
          </IconButton>
        </div>
        <Button
          variant="ghost"
          size="sm"
          :disabled="busy || !selectedSource?.effective?.value"
          @click="copyEffectiveToLayer"
        >
          <RiFileCopyLine class="mr-1.5 h-4 w-4" /> {{ $st('Copy effective catalog') }}
        </Button>
      </div>

      <div v-if="selectedName && selectedConfig" class="grid min-w-0 gap-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div class="grid gap-1">
            <input
              :value="selectedName"
              type="text"
              class="h-9 min-w-[14rem] rounded-md border border-input bg-transparent px-3 font-mono text-sm font-semibold outline-none focus:border-ring"
              :disabled="busy"
              :title="$st('Rename harness')"
              @change="renameHarness"
            />
            <code class="text-[10px] text-muted-foreground">{{ targetLayer }} · {{ settingPath }}</code>
          </div>
          <div class="flex gap-1">
            <Button variant="ghost" size="sm" class="text-destructive" :disabled="busy" @click="removeHarness">
              <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ $st('Delete') }}
            </Button>
            <Button size="sm" :disabled="busy" @click="save">
              <RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? $st('Saving…') : $st('Save layer') }}
            </Button>
          </div>
        </div>

        <div v-if="selectedKind === 'browser'" class="grid gap-3 sm:grid-cols-2">
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Driver') }}</span>
            <Input
              :model-value="browserConfig?.driver || ''"
              :disabled="busy"
              @update:model-value="setBrowserField('driver', $event)"
            />
          </label>
          <label class="inline-flex items-center gap-2 text-sm">
            <input
              :checked="browserConfig?.headless !== false"
              type="checkbox"
              :disabled="busy"
              @change="setBrowserField('headless', ($event.target as HTMLInputElement).checked)"
            />
            {{ $st('Headless') }}
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Viewport width') }}</span>
            <Input
              type="number"
              :model-value="browserConfig?.viewport?.width || 0"
              :disabled="busy"
              @update:model-value="setBrowserField('width', Number($event))"
            />
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Viewport height') }}</span>
            <Input
              type="number"
              :model-value="browserConfig?.viewport?.height || 0"
              :disabled="busy"
              @update:model-value="setBrowserField('height', Number($event))"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">{{ $st('Allowed domains (comma-separated)') }}</span>
            <Input
              :model-value="arrayText(browserConfig?.allowed_domains)"
              class="font-mono"
              :disabled="busy"
              @update:model-value="setBrowserField('allowed_domains', $event)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">{{ $st('Launch options JSON') }}</span>
            <textarea
              :value="jsonText(browserConfig?.launch_options)"
              rows="6"
              spellcheck="false"
              :disabled="busy"
              class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring"
              @change="setBrowserLaunchOptions(($event.target as HTMLTextAreaElement).value)"
            />
          </label>
        </div>

        <div v-else-if="selectedKind === 'shell'" class="grid gap-3 sm:grid-cols-2">
          <label class="inline-flex items-center gap-2 text-sm">
            <input
              :checked="shellConfig?.workspace_only !== false"
              type="checkbox"
              :disabled="busy"
              @change="setShellField('workspace_only', ($event.target as HTMLInputElement).checked)"
            />
            {{ $st('Workspace only') }}
          </label>
          <div></div>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Allowed commands (comma-separated)') }}</span>
            <Input
              :model-value="arrayText(shellConfig?.allow_commands)"
              class="font-mono"
              :disabled="busy"
              @update:model-value="setShellField('allow_commands', $event)"
            />
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Denied commands (comma-separated)') }}</span>
            <Input
              :model-value="arrayText(shellConfig?.deny_commands)"
              class="font-mono"
              :disabled="busy"
              @update:model-value="setShellField('deny_commands', $event)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">{{ $st('Environment (one KEY=VALUE per line)') }}</span>
            <textarea
              :value="envText(shellConfig?.env)"
              rows="7"
              spellcheck="false"
              :disabled="busy"
              class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring"
              @change="setShellEnv(($event.target as HTMLTextAreaElement).value)"
            />
          </label>
        </div>

        <div v-else class="grid gap-3 sm:grid-cols-2">
          <label class="inline-flex items-center gap-2 text-sm">
            <input
              :checked="editorConfig?.workspace_only !== false"
              type="checkbox"
              :disabled="busy"
              @change="setEditorField('workspace_only', ($event.target as HTMLInputElement).checked)"
            />
            {{ $st('Workspace only') }}
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">{{ $st('Max file bytes') }}</span>
            <Input
              type="number"
              :model-value="editorConfig?.max_file_bytes || ''"
              :disabled="busy"
              @update:model-value="setEditorField('max_file_bytes', $event)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">{{ $st('Allowed extensions (comma-separated)') }}</span>
            <Input
              :model-value="arrayText(editorConfig?.allowed_extensions)"
              class="font-mono"
              :disabled="busy"
              @update:model-value="setEditorField('allowed_extensions', $event)"
            />
          </label>
        </div>

        <div class="grid gap-2 border-t border-border/60 pt-3">
          <div class="flex items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">{{ $st('Raw harness JSON') }}</div>
              <div class="mt-1 text-xs text-muted-foreground">
                {{ $st('Edit the complete selected harness object.') }}
              </div>
            </div>
            <Button variant="outline" size="sm" :disabled="busy" @click="applyJson()">{{ $st('Apply JSON') }}</Button>
          </div>
          <textarea
            v-model="rawHarnessJson"
            rows="10"
            spellcheck="false"
            :disabled="busy"
            class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs"
            @input="markRawHarnessJsonDirty"
          />
          <div v-if="jsonError" class="text-xs text-destructive">{{ jsonError }}</div>
        </div>

        <div class="grid gap-2 border-t border-border/60 pt-3 text-[11px] text-muted-foreground">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span>
              {{ selectedLayerSummary }}
            </span>
            <Button variant="ghost" size="sm" :disabled="busy || !selectedLayerResponse?.value" @click="clearKind">
              {{ $st('Clear {layer} {path}', { layer: targetLayerLabel, path: settingPath }) }}
            </Button>
          </div>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-xs font-medium">
              {{ $st('Compare Global, Workspace, and Effective catalogs') }}
            </summary>
            <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[10px] leading-5">{{
              JSON.stringify(
                {
                  global: selectedSource?.global?.value ?? null,
                  workspace: selectedSource?.workspace?.value ?? null,
                  effective: selectedSource?.effective?.value ?? null,
                },
                null,
                2,
              )
            }}</pre>
          </details>
        </div>
      </div>

      <div v-else class="rounded-md border border-dashed border-border/60 px-4 py-8 text-sm text-muted-foreground">
        {{
          $st('Add a harness to the {layer} layer, or copy the current effective catalog before editing.', {
            layer: targetLayerLabel,
          })
        }}
      </div>
    </div>
  </section>
</template>
