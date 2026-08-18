<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  deleteRuntimeSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import type { JsonValue } from '@/types/json'

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
type HarnessMap = Record<string, BrowserHarness | ShellHarness | EditorHarness>

const selectedKind = ref<HarnessKind>('browser')
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

const names = computed(() => Object.keys(maps.value[selectedKind.value] || {}).sort((a, b) => a.localeCompare(b)))
const selectedConfig = computed(() =>
  selectedName.value ? maps.value[selectedKind.value]?.[selectedName.value] : null,
)
const browserConfig = computed(() => selectedConfig.value as BrowserHarness | null)
const shellConfig = computed(() => selectedConfig.value as ShellHarness | null)
const editorConfig = computed(() => selectedConfig.value as EditorHarness | null)
const selectedSource = computed(() => sources.value[selectedKind.value])
const settingPath = computed(() => `harnesses.${selectedKind.value}`)

function asRecord(value: unknown): HarnessMap {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as HarnessMap) : {}
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
  const defaults: Record<HarnessKind, HarnessMap[string]> = {
    browser: { driver: 'playwright', headless: true, viewport: { width: 1280, height: 800 }, allowed_domains: [] },
    shell: { workspace_only: true, allow_commands: [], deny_commands: [], env: {} },
    editor: { workspace_only: true, max_file_bytes: null, allowed_extensions: [] },
  }
  maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [name]: defaults[selectedKind.value] }
  selectedName.value = name
  newName.value = ''
  syncRawHarnessJson()
}

function updateSelected(mutator: (value: any) => void) {
  if (!selectedName.value) return
  const current = JSON.parse(JSON.stringify(selectedConfig.value || {}))
  mutator(current)
  maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [selectedName.value]: current }
  syncRawHarnessJson()
}

function setBrowserField(key: string, value: string | boolean | number) {
  updateSelected((current) => {
    if (key === 'headless') current.headless = value === true
    else if (key === 'width' || key === 'height') {
      current.viewport ||= {}
      current.viewport[key] = Number(value) || 0
    } else if (key === 'allowed_domains') current.allowed_domains = parseList(String(value))
    else current[key] = value
  })
}

function setShellField(key: string, value: string | boolean) {
  updateSelected((current) => {
    if (key === 'workspace_only') current.workspace_only = value === true
    else if (key === 'allow_commands' || key === 'deny_commands') current[key] = parseList(String(value))
    else current[key] = value
  })
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
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error('Harness config must be a JSON object.')
    if (!selectedName.value) return
    maps.value[selectedKind.value] = { ...maps.value[selectedKind.value], [selectedName.value]: parsed as any }
    syncRawHarnessJson()
    jsonError.value = ''
  } catch (reason) {
    jsonError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const next: Partial<Record<HarnessKind, RuntimeSettingsReadBundle>> = {}
    for (const kind of ['browser', 'shell', 'editor'] as HarnessKind[]) {
      next[kind] = await readRuntimeSettingSources(`harnesses.${kind}`)
      maps.value[kind] = asRecord(next[kind]?.effective.value)
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
    await setRuntimeSetting(settingPath.value, maps.value[selectedKind.value] as JsonValue, { reload: true }, 'global')
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
  selectedName.value = Object.keys(next)[0] || ''
  await save()
}

async function clearKind() {
  if (!window.confirm(`Clear ${settingPath.value}?`)) return
  try {
    await deleteRuntimeSetting(settingPath.value, { reload: true }, 'global')
    await load()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

onMounted(() => void load())
watch(selectedKind, () => {
  // The TUI changes the visible harness catalog with the selected kind. Keep
  // the editor on a real harness when the previous kind's name does not
  // exist in the new catalog instead of leaving a misleading empty editor.
  if (!names.value.includes(selectedName.value)) selectedName.value = names.value[0] || ''
  syncRawHarnessJson()
})
watch(selectedName, () => syncRawHarnessJson())
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 bg-background/30 p-4 lg:p-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-medium">Browser / Shell / Editor Harnesses</h2>
        <p class="mt-1 text-xs text-muted-foreground">
          These are the three harness configuration paths exposed in the TUI Plugins &amp; Tools section.
        </p>
      </div>
      <Button variant="outline" size="sm" :disabled="loading" @click="load"
        ><RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" /> Refresh</Button
      >
    </div>
    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <div class="grid gap-4 lg:grid-cols-[minmax(13rem,0.7fr)_minmax(0,2fr)]">
      <div class="grid content-start gap-2">
        <OptionPicker v-model="selectedKind" :options="kindOptions" :include-empty="false" title="Harness kind" />
        <button
          v-for="name in names"
          :key="name"
          type="button"
          class="rounded-md border px-3 py-2 text-left text-xs"
          :class="selectedName === name ? 'border-primary bg-primary/10' : 'border-border/60 hover:bg-muted/40'"
          @click="selectName(name)"
        >
          <code>{{ name }}</code>
        </button>
        <div
          v-if="names.length === 0"
          class="rounded-md border border-dashed border-border/60 px-3 py-4 text-center text-xs text-muted-foreground"
        >
          No harnesses configured.
        </div>
        <div class="flex gap-2">
          <Input
            v-model="newName"
            class="min-w-0 font-mono"
            placeholder="default"
            @keydown.enter="addHarness"
          /><IconButton
            variant="outline"
            size="sm"
            tooltip="Add harness"
            aria-label="Add harness"
            :disabled="!newName.trim()"
            @click="addHarness"
            ><RiAddLine class="h-4 w-4"
          /></IconButton>
        </div>
      </div>
      <div v-if="selectedName && selectedConfig" class="grid min-w-0 gap-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div>
            <div class="font-mono text-sm font-semibold">{{ selectedKind }} / {{ selectedName }}</div>
            <code class="text-[10px] text-muted-foreground">{{ settingPath }}</code>
          </div>
          <div class="flex gap-1">
            <Button variant="ghost" size="sm" class="text-destructive" @click="removeHarness"
              ><RiDeleteBinLine class="mr-1.5 h-4 w-4" /> Delete</Button
            ><Button size="sm" :disabled="saving" @click="save"
              ><RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? 'Saving…' : 'Save' }}</Button
            >
          </div>
        </div>
        <div v-if="selectedKind === 'browser'" class="grid gap-3 sm:grid-cols-2">
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Driver</span
            ><Input
              :value="browserConfig?.driver || ''"
              @input="setBrowserField('driver', ($event.target as HTMLInputElement).value)"
          /></label>
          <label class="inline-flex items-center gap-2 text-sm"
            ><input
              :checked="browserConfig?.headless !== false"
              type="checkbox"
              @change="setBrowserField('headless', ($event.target as HTMLInputElement).checked)"
            />
            Headless</label
          >
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Viewport width</span
            ><Input
              type="number"
              :value="browserConfig?.viewport?.width || 0"
              @input="setBrowserField('width', Number(($event.target as HTMLInputElement).value))"
          /></label>
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Viewport height</span
            ><Input
              type="number"
              :value="browserConfig?.viewport?.height || 0"
              @input="setBrowserField('height', Number(($event.target as HTMLInputElement).value))"
          /></label>
          <label class="grid gap-1.5 sm:col-span-2"
            ><span class="text-xs text-muted-foreground">Allowed domains (comma-separated)</span
            ><Input
              :value="arrayText(browserConfig?.allowed_domains)"
              class="font-mono"
              @input="setBrowserField('allowed_domains', ($event.target as HTMLInputElement).value)"
          /></label>
        </div>
        <div v-else-if="selectedKind === 'shell'" class="grid gap-3 sm:grid-cols-2">
          <label class="inline-flex items-center gap-2 text-sm"
            ><input
              :checked="shellConfig?.workspace_only !== false"
              type="checkbox"
              @change="setShellField('workspace_only', ($event.target as HTMLInputElement).checked)"
            />
            Workspace only</label
          >
          <div></div>
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Allowed commands</span
            ><Input
              :value="arrayText(shellConfig?.allow_commands)"
              class="font-mono"
              @input="setShellField('allow_commands', ($event.target as HTMLInputElement).value)"
          /></label>
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Denied commands</span
            ><Input
              :value="arrayText(shellConfig?.deny_commands)"
              class="font-mono"
              @input="setShellField('deny_commands', ($event.target as HTMLInputElement).value)"
          /></label>
        </div>
        <div v-else class="grid gap-3 sm:grid-cols-2">
          <label class="inline-flex items-center gap-2 text-sm"
            ><input
              :checked="editorConfig?.workspace_only !== false"
              type="checkbox"
              @change="setEditorField('workspace_only', ($event.target as HTMLInputElement).checked)"
            />
            Workspace only</label
          >
          <label class="grid gap-1.5"
            ><span class="text-xs text-muted-foreground">Max file bytes</span
            ><Input
              type="number"
              :value="editorConfig?.max_file_bytes || ''"
              @input="setEditorField('max_file_bytes', ($event.target as HTMLInputElement).value)"
          /></label>
          <label class="grid gap-1.5 sm:col-span-2"
            ><span class="text-xs text-muted-foreground">Allowed extensions (comma-separated)</span
            ><Input
              :value="arrayText(editorConfig?.allowed_extensions)"
              class="font-mono"
              @input="setEditorField('allowed_extensions', ($event.target as HTMLInputElement).value)"
          /></label>
        </div>
        <div class="grid gap-2 border-t border-border/60 pt-3">
          <div class="flex items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">Raw harness JSON</div>
              <div class="mt-1 text-xs text-muted-foreground">
                Use JSON for launch options or environment entries that are not expanded above.
              </div>
            </div>
            <Button variant="outline" size="sm" @click="applyJson()">Validate current JSON</Button>
          </div>
          <textarea
            v-model="rawHarnessJson"
            rows="10"
            spellcheck="false"
            class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs"
          />
          <div v-if="jsonError" class="text-xs text-destructive">{{ jsonError }}</div>
        </div>
        <div
          class="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 pt-3 text-[11px] text-muted-foreground"
        >
          <span
            >Effective:
            {{
              selectedSource?.effective?.value !== undefined && selectedSource?.effective?.value !== null
                ? JSON.stringify(selectedSource.effective.value)
                : '—'
            }}</span
          ><Button variant="ghost" size="sm" @click="clearKind">Clear {{ settingPath }}</Button>
        </div>
      </div>
      <div v-else class="rounded-md border border-dashed border-border/60 px-4 py-8 text-sm text-muted-foreground">
        Add or select a harness to edit it.
      </div>
    </div>
  </section>
</template>
