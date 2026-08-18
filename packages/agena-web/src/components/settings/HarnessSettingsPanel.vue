<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
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

const kindOptions = [
  { value: 'browser', label: 'Browser Harness', description: 'Browser driver, domains, viewport, and launch options.' },
  {
    value: 'shell',
    label: 'Shell Harness',
    description: 'Workspace boundary, command allow/deny lists, and environment.',
  },
  { value: 'editor', label: 'Editor Harness', description: 'Workspace boundary, file size, and extension allowlist.' },
]
const layerOptions = [
  { value: 'global', label: 'Global layer', description: 'Available to all workspaces.' },
  { value: 'workspace', label: 'Workspace layer', description: 'Overrides only the current workspace.' },
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
const busy = computed(() => loading.value || saving.value)

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
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
    if (separator <= 0) throw new Error(`Environment line ${index + 1} must use KEY=VALUE.`)
    const key = line.slice(0, separator).trim()
    if (!key) throw new Error(`Environment line ${index + 1} has an empty key.`)
    output[key] = line.slice(separator + 1)
  }
  return output
}

function sourceValue(bundle: RuntimeSettingsReadBundle, layer: RuntimeSettingsLayer): JsonValue {
  return layer === 'workspace' ? bundle.workspace.value : bundle.global.value
}

function syncRawHarnessJson() {
  rawHarnessJson.value = selectedConfig.value ? JSON.stringify(selectedConfig.value, null, 2) : '{}'
  jsonError.value = ''
}

function selectName(name: string) {
  selectedName.value = name
  syncRawHarnessJson()
}

function addHarness() {
  const name = newName.value.trim()
  if (!name) return
  if (Object.prototype.hasOwnProperty.call(maps.value[selectedKind.value], name)) {
    error.value = `Harness already exists in the ${targetLayer.value} layer: ${name}`
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
    error.value = `Harness already exists in the ${targetLayer.value} layer: ${nextName}`
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
  const current = clone((selectedConfig.value || {}) as JsonObject)
  mutator(current)
  maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [selectedName.value]: current as HarnessConfig }
  syncRawHarnessJson()
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
      throw new Error('Harness config must be a JSON object.')
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
  if (!selectedName.value || !window.confirm(`Delete ${selectedKind.value} harness ${selectedName.value}?`)) return
  const next = { ...maps.value[selectedKind.value] }
  delete next[selectedName.value]
  maps.value[selectedKind.value] = next
  selectedName.value = Object.keys(next).sort()[0] || ''
  await save()
}

async function clearKind() {
  if (!window.confirm(`Clear ${targetLayer.value} ${settingPath.value}?`)) return
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
</script>

<template>
  <section class="grid gap-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">Browser / Shell / Editor Harnesses</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">
          Edit the selected configuration layer explicitly. Effective values remain visible for comparison and can be
          copied into the current layer without silently promoting Workspace overrides to Global.
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <div class="w-48">
          <OptionPicker
            v-model="targetLayer"
            :options="layerOptions"
            :include-empty="false"
            title="Harness settings layer"
            :disabled="busy"
          />
        </div>
        <Button variant="outline" size="sm" :disabled="busy" @click="load">
          <RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" /> Refresh
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
          title="Harness kind"
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
          No harnesses configured in the {{ targetLayer }} layer.
        </div>
        <div class="flex gap-2">
          <Input
            v-model="newName"
            class="min-w-0 font-mono"
            placeholder="default"
            :disabled="busy"
            @keydown.enter="addHarness"
          />
          <IconButton
            variant="outline"
            size="sm"
            tooltip="Add harness"
            aria-label="Add harness"
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
          <RiFileCopyLine class="mr-1.5 h-4 w-4" /> Copy effective catalog
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
              title="Rename harness"
              @change="renameHarness"
            />
            <code class="text-[10px] text-muted-foreground">{{ targetLayer }} · {{ settingPath }}</code>
          </div>
          <div class="flex gap-1">
            <Button variant="ghost" size="sm" class="text-destructive" :disabled="busy" @click="removeHarness">
              <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> Delete
            </Button>
            <Button size="sm" :disabled="busy" @click="save">
              <RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? 'Saving…' : 'Save layer' }}
            </Button>
          </div>
        </div>

        <div v-if="selectedKind === 'browser'" class="grid gap-3 sm:grid-cols-2">
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Driver</span>
            <Input
              :value="browserConfig?.driver || ''"
              :disabled="busy"
              @input="setBrowserField('driver', ($event.target as HTMLInputElement).value)"
            />
          </label>
          <label class="inline-flex items-center gap-2 text-sm">
            <input
              :checked="browserConfig?.headless !== false"
              type="checkbox"
              :disabled="busy"
              @change="setBrowserField('headless', ($event.target as HTMLInputElement).checked)"
            />
            Headless
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Viewport width</span>
            <Input
              type="number"
              :value="browserConfig?.viewport?.width || 0"
              :disabled="busy"
              @input="setBrowserField('width', Number(($event.target as HTMLInputElement).value))"
            />
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Viewport height</span>
            <Input
              type="number"
              :value="browserConfig?.viewport?.height || 0"
              :disabled="busy"
              @input="setBrowserField('height', Number(($event.target as HTMLInputElement).value))"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">Allowed domains (comma-separated)</span>
            <Input
              :value="arrayText(browserConfig?.allowed_domains)"
              class="font-mono"
              :disabled="busy"
              @input="setBrowserField('allowed_domains', ($event.target as HTMLInputElement).value)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">Launch options JSON</span>
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
            Workspace only
          </label>
          <div></div>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Allowed commands (comma-separated)</span>
            <Input
              :value="arrayText(shellConfig?.allow_commands)"
              class="font-mono"
              :disabled="busy"
              @input="setShellField('allow_commands', ($event.target as HTMLInputElement).value)"
            />
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Denied commands (comma-separated)</span>
            <Input
              :value="arrayText(shellConfig?.deny_commands)"
              class="font-mono"
              :disabled="busy"
              @input="setShellField('deny_commands', ($event.target as HTMLInputElement).value)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">Environment (one KEY=VALUE per line)</span>
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
            Workspace only
          </label>
          <label class="grid gap-1.5">
            <span class="text-xs text-muted-foreground">Max file bytes</span>
            <Input
              type="number"
              :value="editorConfig?.max_file_bytes || ''"
              :disabled="busy"
              @input="setEditorField('max_file_bytes', ($event.target as HTMLInputElement).value)"
            />
          </label>
          <label class="grid gap-1.5 sm:col-span-2">
            <span class="text-xs text-muted-foreground">Allowed extensions (comma-separated)</span>
            <Input
              :value="arrayText(editorConfig?.allowed_extensions)"
              class="font-mono"
              :disabled="busy"
              @input="setEditorField('allowed_extensions', ($event.target as HTMLInputElement).value)"
            />
          </label>
        </div>

        <div class="grid gap-2 border-t border-border/60 pt-3">
          <div class="flex items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">Raw harness JSON</div>
              <div class="mt-1 text-xs text-muted-foreground">Edit the complete selected harness object.</div>
            </div>
            <Button variant="outline" size="sm" :disabled="busy" @click="applyJson()">Apply JSON</Button>
          </div>
          <textarea
            v-model="rawHarnessJson"
            rows="10"
            spellcheck="false"
            :disabled="busy"
            class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs"
          />
          <div v-if="jsonError" class="text-xs text-destructive">{{ jsonError }}</div>
        </div>

        <div class="grid gap-2 border-t border-border/60 pt-3 text-[11px] text-muted-foreground">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span>
              Editing {{ targetLayer }} layer ·
              {{
                selectedLayerResponse?.value
                  ? `${Object.keys(asRecord(selectedLayerResponse.value)).length} harnesses`
                  : 'unset'
              }}
            </span>
            <Button variant="ghost" size="sm" :disabled="busy || !selectedLayerResponse?.value" @click="clearKind">
              Clear {{ targetLayer }} {{ settingPath }}
            </Button>
          </div>
          <details class="rounded-md border border-border/60">
            <summary class="cursor-pointer px-3 py-2 text-xs font-medium">
              Compare Global, Workspace, and Effective catalogs
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
        Add a harness to the {{ targetLayer }} layer, or copy the current effective catalog before editing.
      </div>
    </div>
  </section>
</template>
