<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router'
import { RiArrowDownLine, RiArrowUpLine, RiDeleteBinLine, RiRefreshLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { confirmAction } from '@/lib/appConfirm'
import {
  MAX_PROMPT_TOTAL_BYTES,
  clonePromptLayer,
  emptyPromptLayer,
  emptyPromptScopeConfig,
  normalizePromptScopeConfig,
  promptFragmentBodyBytes,
  promptLayerFromScopeConfig,
  promptLayerAddOrder,
  promptLayerHasErrors,
  promptLayerMoveOrder,
  promptLayerRemoveOrder,
  promptLayerSetDisabled,
  promptLayersEqual,
  promptScopeConfigRemoveFragment,
  promptScopeConfigUpdateFragment,
  promptScopeConfigWithLayer,
  promptScopeConfigIsEmpty,
  validatePromptLayer,
  type PromptFragment,
  type PromptLayer,
  type PromptLayerScope,
  type PromptResolution,
  type PromptScopeConfig,
} from '@/lib/promptLayers'
import { availableSavedPrompts, savedPromptDisplayName } from '@/lib/promptLibrary'
import { deleteRuntimeSetting, getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import { getSessionExecution, getSessionPromptPreview, setSessionPrompts } from '@/stores/chat/api'
import { useChatStore } from '@/stores/chat'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'

type LoadedLayer = {
  layer: PromptLayer
  config: PromptScopeConfig
}

type CachedDraft = LoadedLayer & {
  baseline: PromptLayer
  savedConfig: PromptScopeConfig
}

const SETTING_PATH = 'prompts'
const props = defineProps<{ scope: string }>()
const route = useRoute()
const router = useRouter()
const chat = useChatStore()
const directory = useDirectoryStore()
const toasts = useToastsStore()

const scope = computed<PromptLayerScope>(() =>
  props.scope === 'global' || props.scope === 'workspace' || props.scope === 'session' ? props.scope : 'effective',
)
const sessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? String(id) : null
})
const needsSession = computed(() => scope.value === 'session' || scope.value === 'effective')
const contextAvailable = computed(() => !needsSession.value || sessionId.value !== null)
const contextKey = computed(() =>
  JSON.stringify([
    scope.value,
    needsSession.value ? sessionId.value : null,
    scope.value === 'workspace' ? directory.currentDirectory : null,
  ]),
)

const layer = ref<PromptLayer>(emptyPromptLayer())
const baseline = ref<PromptLayer>(emptyPromptLayer())
const config = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const savedConfig = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const globalSaved = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const workspaceSaved = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const preview = ref<PromptResolution | null>(null)
const loaded = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const previewError = ref('')
const orderedFragmentInput = ref('')
const disabledFragmentInput = ref('')
const drafts = new Map<string, CachedDraft>()
let loadedContext = ''
let loadGeneration = 0

const dirty = computed(
  () => scope.value !== 'effective' && loaded.value && !promptLayersEqual(layer.value, baseline.value),
)
const issues = computed(() => (scope.value === 'effective' ? [] : validatePromptLayer(layer.value, true)))
const valid = computed(() => !promptLayerHasErrors(issues.value))
const inlineById = computed(() => new Map(layer.value.inline.map((fragment) => [fragment.id, fragment])))
const available = computed(() => availableSavedPrompts(globalSaved.value, workspaceSaved.value, scope.value))
const availableById = computed(() => new Map(available.value.map((item) => [item.id, item])))
const addable = computed(() =>
  available.value.filter((item) =>
    !layer.value.order.includes(item.id) && !layer.value.disabled.includes(item.id) && !inlineById.value.has(item.id),
  ),
)
const hideable = computed(() =>
  available.value.filter((item) =>
    scope.value !== 'global' &&
    (scope.value !== 'workspace' || item.scope === 'global') &&
    !layer.value.disabled.includes(item.id) &&
    !layer.value.order.includes(item.id) &&
    !inlineById.value.has(item.id),
  ),
)
const orderedEntries = computed(() =>
  layer.value.order.map((id, index) => ({
    id,
    index,
    inlineIndex: layer.value.inline.findIndex((fragment) => fragment.id === id),
    fragment: inlineById.value.get(id),
    saved: availableById.value.get(id),
  })),
)
const unorderedInline = computed(() =>
  layer.value.inline.filter((fragment) => !layer.value.order.includes(fragment.id)),
)
const editingDisabled = computed(() => !loaded.value || loading.value || saving.value || !contextAvailable.value)
const modeOptions = computed(() => [
  {
    value: 'append',
    label: st('Append'),
    description: st('Keep broader instructions, then add this layer.'),
  },
  {
    value: 'replace',
    label: st('Replace'),
    description: st('Discard broader instructions before adding this layer.'),
  },
])
const requiresToolsOptions = computed(() => [
  { value: '', label: st('Inherit'), description: st('Do not condition this fragment on tool availability.') },
  { value: 'true', label: st('Required'), description: st('Only apply when the session exposes tools.') },
  { value: 'false', label: st('Not required'), description: st('Only apply when the session exposes no tools.') },
])
const issueLabels: Record<string, string> = {
  invalid_id: st('Use 1–64 lowercase letters, digits, dots, underscores, or hyphens; the id cannot start with a dot.'),
  duplicate_order: st('This fragment id appears more than once in the order.'),
  duplicate_inline: st('This inline fragment id is defined more than once.'),
  duplicate_disabled: st('This disabled fragment id appears more than once.'),
  order_disabled_conflict: st('A fragment cannot be both ordered and disabled.'),
  unknown_inline_fragment: st('This ordered fragment is not defined inline.'),
  external_order_fragment: st('This ordered fragment must exist in the broader fragment library.'),
  inline_not_in_order: st('This inline fragment is disabled until it is added to the order.'),
  fragment_too_large: st('This fragment exceeds the 16 KiB limit.'),
  empty_body: st('This fragment is empty.'),
}

function issueLabel(code: string): string {
  return issueLabels[code] || code
}

function rememberDraft() {
  if (scope.value === 'effective' || !loaded.value || !loadedContext) return
  if (dirty.value) {
    drafts.set(loadedContext, {
      layer: clonePromptLayer(layer.value),
      config: normalizePromptScopeConfig(config.value),
      baseline: clonePromptLayer(baseline.value),
      savedConfig: normalizePromptScopeConfig(savedConfig.value),
    })
  } else {
    drafts.delete(loadedContext)
  }
}

async function readLayer(source: PromptLayerScope, id: string | null): Promise<LoadedLayer | null> {
  if (source === 'global' || source === 'workspace') {
    const response = await getRuntimeSettingLayer(source, SETTING_PATH)
    const loadedConfig = normalizePromptScopeConfig(response.value)
    return { layer: clonePromptLayer(loadedConfig), config: loadedConfig }
  }
  if (!id) return null
  const state = await getSessionExecution(id)
  return { layer: clonePromptLayer(state.execution?.prompts || emptyPromptLayer()), config: emptyPromptScopeConfig() }
}

async function load() {
  rememberDraft()
  const key = contextKey.value
  const source = scope.value
  const id = sessionId.value
  const generation = ++loadGeneration
  loading.value = true
  loaded.value = false
  error.value = ''
  previewError.value = ''
  try {
    if (source === 'effective') {
      if (!id) return
      const result = await getSessionPromptPreview(id)
      if (generation !== loadGeneration || key !== contextKey.value) return
      preview.value = result
      loadedContext = key
      loaded.value = true
      return
    }
    const [fetched, globalResponse, workspaceResponse] = await Promise.all([
      readLayer(source, id),
      getRuntimeSettingLayer('global', SETTING_PATH),
      getRuntimeSettingLayer('workspace', SETTING_PATH),
    ])
    if (generation !== loadGeneration || key !== contextKey.value) return
    globalSaved.value = normalizePromptScopeConfig(globalResponse.value)
    workspaceSaved.value = normalizePromptScopeConfig(workspaceResponse.value)
    const cached = drafts.get(key)
    layer.value = clonePromptLayer(cached?.layer || fetched?.layer || emptyPromptLayer())
    config.value = normalizePromptScopeConfig(cached?.config || fetched?.config || emptyPromptScopeConfig())
    baseline.value = clonePromptLayer(cached?.baseline || fetched?.layer || emptyPromptLayer())
    savedConfig.value = normalizePromptScopeConfig(cached?.savedConfig || fetched?.config || emptyPromptScopeConfig())
    loadedContext = key
    loaded.value = true
  } catch (reason) {
    if (generation === loadGeneration && key === contextKey.value) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (generation === loadGeneration) loading.value = false
  }
}

function discard() {
  layer.value = clonePromptLayer(baseline.value)
  config.value = normalizePromptScopeConfig(savedConfig.value)
  drafts.delete(loadedContext)
  error.value = ''
}

function setMode(value: string) {
  if (value !== 'append' && value !== 'replace') return
  layer.value = { ...clonePromptLayer(layer.value), mode: value }
}

function openSavedLibrary() {
  void router.push({
    path: route.path,
    query: { ...route.query, view: 'prompt-library', scope: scope.value === 'global' ? 'global' : 'workspace' },
  })
}

function updateFragment(index: number, fragment: PromptFragment) {
  if (index < 0) return
  const nextConfig = promptScopeConfigUpdateFragment(
    promptScopeConfigWithLayer(config.value, layer.value),
    index,
    fragment,
  )
  config.value = nextConfig
  layer.value = promptLayerFromScopeConfig(nextConfig)
}

function updateFragmentField(index: number, field: 'id' | 'title' | 'body', value: string) {
  const fragment = layer.value.inline[index]
  if (!fragment) return
  updateFragment(index, { ...fragment, [field]: value })
}

function updateFragmentModels(index: number, value: string) {
  const fragment = layer.value.inline[index]
  if (!fragment) return
  updateFragment(index, {
    ...fragment,
    when: {
      ...fragment.when,
      models: value
        .split(',')
        .map((model) => model.trim())
        .filter(Boolean),
    },
  })
}

function updateFragmentTools(index: number, value: string) {
  const fragment = layer.value.inline[index]
  if (!fragment) return
  updateFragment(index, {
    ...fragment,
    when: { ...fragment.when, requires_tools: value === 'true' ? true : value === 'false' ? false : null },
  })
}

function removeFragment(id: string) {
  const nextConfig = promptScopeConfigRemoveFragment(promptScopeConfigWithLayer(config.value, layer.value), id)
  config.value = nextConfig
  layer.value = promptLayerFromScopeConfig(nextConfig)
}

function removeOrder(id: string) {
  layer.value = promptLayerRemoveOrder(layer.value, id)
}

function moveOrder(id: string, delta: number) {
  layer.value = promptLayerMoveOrder(layer.value, id, delta)
}

function addOrderedFragment() {
  const id = orderedFragmentInput.value.trim()
  if (!id) return
  layer.value = promptLayerAddOrder(layer.value, id)
  orderedFragmentInput.value = ''
}

function addDisabledFragment() {
  const id = disabledFragmentInput.value.trim()
  if (!id) return
  layer.value = promptLayerSetDisabled(layer.value, id, true)
  disabledFragmentInput.value = ''
}

function enableFragment(id: string) {
  layer.value = promptLayerSetDisabled(layer.value, id, false)
}

async function save() {
  if (editingDisabled.value || scope.value === 'effective' || !dirty.value || !valid.value) return
  const source = scope.value
  const id = sessionId.value
  const key = contextKey.value
  saving.value = true
  error.value = ''
  try {
    if (source === 'session') {
      if (!id) return
      await setSessionPrompts(id, clonePromptLayer(layer.value))
    } else {
      if (source !== 'global' && source !== 'workspace') return
      // The saved library may have changed in another settings view while
      // this layer was being edited. Rebase layer changes on the latest
      // catalog instead of overwriting reusable saved prompts.
      const latest = normalizePromptScopeConfig((await getRuntimeSettingLayer(source, SETTING_PATH)).value)
      const modelsEdited = JSON.stringify(config.value.models) !== JSON.stringify(savedConfig.value.models)
      const nextConfig = promptScopeConfigWithLayer(
        { ...latest, models: modelsEdited ? config.value.models : latest.models },
        layer.value,
      )
      if (promptScopeConfigIsEmpty(nextConfig)) {
        await deleteRuntimeSetting(SETTING_PATH, undefined, source)
      } else {
        await setRuntimeSetting(SETTING_PATH, nextConfig as unknown as JsonValue, undefined, source)
      }
    }
    drafts.delete(key)
    loaded.value = false
    await load()
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    if (key === contextKey.value) error.value = message
    else toasts.push('error', message)
  } finally {
    saving.value = false
  }
}

async function refreshPreview() {
  const id = sessionId.value
  if (!id || loading.value) return
  previewError.value = ''
  try {
    preview.value = await getSessionPromptPreview(id)
  } catch (reason) {
    previewError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function canLeave() {
  if (saving.value) return false
  rememberDraft()
  if (!dirty.value && !drafts.size) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

onBeforeRouteUpdate(async (to, from) => {
  if (saving.value) return false
  // Scope selection is a query update that keeps this page mounted. Preserve
  // its draft cache instead of treating it like leaving the page.
  if (to.path === from.path && String(to.query.view ?? '') === String(from.query.view ?? '')) {
    rememberDraft()
    return true
  }
  if (!(await canLeave())) return false
  discard()
  drafts.clear()
  return true
})
onBeforeRouteLeave(canLeave)

function beforeUnload(event: BeforeUnloadEvent) {
  rememberDraft()
  if (!dirty.value && !drafts.size) return
  event.preventDefault()
  event.returnValue = ''
}

async function refresh() {
  if (!saving.value) await load()
}

defineExpose({ refresh })
watch(contextKey, () => void load())
onMounted(() => {
  void load()
  window.addEventListener('beforeunload', beforeUnload)
})
onBeforeUnmount(() => {
  loadGeneration += 1
  window.removeEventListener('beforeunload', beforeUnload)
})
</script>

<template>
  <div class="grid min-w-0 gap-4" :aria-busy="loading || saving">
    <SettingsCard
      :title="$st('Prompt layers')"
      :description="
        needsSession
          ? $st('Inspect resolved instructions and edit the selected session layer.')
          : $st('Edit ordered instruction fragments for this configuration scope.')
      "
    >
      <p v-if="needsSession && sessionId" class="mb-3 text-xs text-muted-foreground">
        {{ $st('Selected session: {id}', { id: sessionId }) }}
      </p>
      <p v-if="!contextAvailable" class="text-sm text-muted-foreground">
        {{ $st('Select a session to inspect or edit its prompt layers.') }}
      </p>
      <p v-else-if="loading" class="text-sm text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
      <p v-else-if="error" class="break-words text-xs text-destructive" role="alert">{{ error }}</p>

      <div v-else-if="loaded && scope === 'effective'" class="grid min-w-0 gap-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <p class="text-xs leading-relaxed text-muted-foreground">
            {{
              $st(
                'Prompt layers resolve from global through workspace, model, session, and request scopes. Built-in instructions are not included.',
              )
            }}
          </p>
          <Button variant="outline" size="sm" :disabled="loading" @click="refreshPreview">
            <RiRefreshLine class="mr-1.5 h-4 w-4" />
            {{ $st('Refresh preview') }}
          </Button>
        </div>
        <p v-if="previewError" class="break-words text-xs text-destructive" role="alert">{{ previewError }}</p>
        <template v-if="preview">
          <div class="flex flex-wrap items-center gap-2 text-xs">
            <span class="rounded-full border border-border/70 px-2.5 py-1 text-muted-foreground">
              {{
                $st('Prompt size: {used} / {limit} bytes', { used: preview.total_bytes, limit: MAX_PROMPT_TOTAL_BYTES })
              }}
            </span>
            <span
              v-if="preview.over_budget"
              class="rounded-full border border-destructive/40 bg-destructive/10 px-2.5 py-1 font-medium text-destructive"
            >
              {{ $st('Over budget') }}
            </span>
          </div>
          <div class="grid min-w-0 gap-2">
            <h3 class="text-sm font-medium">{{ $st('Resolved prompt') }}</h3>
            <pre
              v-if="preview.text"
              class="max-h-[28rem] overflow-auto whitespace-pre-wrap break-words rounded-md border border-border/60 bg-muted/10 p-3 font-sans text-sm leading-relaxed"
              >{{ preview.text }}</pre
            >
            <p v-else class="text-sm text-muted-foreground">{{ $st('No resolved prompt.') }}</p>
          </div>
          <div class="grid min-w-0 gap-2">
            <h3 class="text-sm font-medium">{{ $st('Resolved fragments') }}</h3>
            <p v-if="!preview.fragments.length" class="text-sm text-muted-foreground">
              {{ $st('No fragments contribute to this prompt.') }}
            </p>
            <details
              v-for="fragment in preview.fragments"
              :key="fragment.position + ':' + fragment.scope + ':' + fragment.id"
              class="min-w-0 rounded-md border border-border/60"
            >
              <summary class="cursor-pointer px-3 py-2 text-sm">
                <span class="font-mono">{{ fragment.id }}</span>
                <span class="ml-2 text-xs text-muted-foreground"
                  >{{ fragment.scope }} · {{ fragment.bytes }} {{ $st('bytes') }}</span
                >
              </summary>
              <pre
                class="overflow-auto whitespace-pre-wrap break-words border-t border-border/60 p-3 font-sans text-xs leading-relaxed"
                >{{ fragment.text }}</pre
              >
            </details>
          </div>
          <div class="grid min-w-0 gap-2">
            <h3 class="text-sm font-medium">{{ $st('Warnings') }}</h3>
            <p v-if="!preview.warnings.length" class="text-sm text-muted-foreground">{{ $st('No warnings.') }}</p>
            <p
              v-for="warning in preview.warnings"
              :key="warning.scope + ':' + warning.id + ':' + warning.kind"
              class="break-words rounded-md border border-amber-500/30 bg-amber-500/[0.06] px-3 py-2 text-xs"
            >
              <span class="font-mono">{{ warning.id }}</span> · {{ warning.detail }}
            </p>
          </div>
        </template>
      </div>

      <div v-else-if="loaded" class="grid min-w-0 gap-5">
        <OptionPicker
          :model-value="layer.mode"
          :options="modeOptions"
          :include-empty="false"
          :title="$st('Mode')"
          :disabled="editingDisabled"
          @update:model-value="setMode"
        />

        <div v-if="issues.length" class="grid gap-1.5 rounded-md border border-amber-500/35 bg-amber-500/[0.06] p-3">
          <p
            v-for="(issue, index) in issues"
            :key="issue.code + ':' + issue.id + ':' + index"
            class="break-words text-xs"
            :class="issue.level === 'error' ? 'text-destructive' : 'text-amber-800 dark:text-amber-200'"
          >
            <span v-if="issue.id" class="font-mono">{{ issue.id }}</span>
            <span v-if="issue.id"> · </span>{{ issueLabel(issue.code) }}
          </p>
        </div>

        <div class="grid min-w-0 gap-2">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h3 class="text-sm font-medium">{{ $st('Ordered fragments') }}</h3>
            <Button variant="outline" size="sm" @click="openSavedLibrary">
              {{ $st('Manage saved prompts') }}
            </Button>
          </div>
          <p v-if="!layer.order.length" class="text-sm text-muted-foreground">
            {{ $st('No prompt fragments in this layer.') }}
          </p>
          <p v-if="!available.length" class="text-xs text-muted-foreground">
            {{ $st('Create prompts in the saved library first, then select them here.') }}
          </p>
          <div
            v-for="entry in orderedEntries"
            :key="entry.index"
            class="grid min-w-0 gap-3 rounded-lg border border-border/60 p-3"
          >
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="flex min-w-0 items-center gap-2">
                <span class="text-xs text-muted-foreground">{{ entry.index + 1 }}</span>
                <span class="truncate text-sm font-medium">{{ entry.fragment?.title || entry.saved?.fragment.title || entry.id }}</span>
                <span
                  v-if="!entry.fragment && !entry.saved"
                  class="rounded-full border border-border/70 px-2 py-0.5 text-[11px] text-muted-foreground"
                >
                  {{ $st('Library fragment') }}
                </span>
                <span v-else-if="entry.saved" class="text-xs text-muted-foreground">
                  {{ entry.saved.fragment.category || entry.saved.scope }}
                </span>
              </div>
              <div class="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  :disabled="editingDisabled || entry.index === 0"
                  @click="moveOrder(entry.id, -1)"
                >
                  <RiArrowUpLine class="h-4 w-4" />
                  <span class="sr-only">{{ $st('Move up') }}</span>
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  :disabled="editingDisabled || entry.index + 1 === layer.order.length"
                  @click="moveOrder(entry.id, 1)"
                >
                  <RiArrowDownLine class="h-4 w-4" />
                  <span class="sr-only">{{ $st('Move down') }}</span>
                </Button>
                <Button variant="ghost" size="sm" :disabled="editingDisabled" @click="removeOrder(entry.id)">
                  <RiDeleteBinLine class="h-4 w-4" />
                  <span class="sr-only">{{ $st('Remove') }}</span>
                </Button>
              </div>
            </div>
            <details v-if="!entry.fragment && entry.saved" class="text-xs text-muted-foreground">
              <summary class="cursor-pointer">{{ $st('Preview instructions') }}</summary>
              <pre class="mt-2 whitespace-pre-wrap break-words font-sans">{{ entry.saved.fragment.body }}</pre>
            </details>

            <div v-if="entry.fragment" class="grid min-w-0 gap-3 md:grid-cols-2">
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('Fragment ID') }}
                <input
                  :value="entry.fragment?.id || ''"
                  :disabled="editingDisabled"
                  spellcheck="false"
                  class="rounded-md border border-input bg-transparent px-2.5 py-1.5 font-mono text-xs text-foreground outline-none focus:border-ring"
                  @change="updateFragmentField(entry.inlineIndex, 'id', ($event.target as HTMLInputElement).value)"
                />
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('Title') }}
                <input
                  :value="entry.fragment?.title || ''"
                  :disabled="editingDisabled"
                  class="rounded-md border border-input bg-transparent px-2.5 py-1.5 text-sm text-foreground outline-none focus:border-ring"
                  @input="updateFragmentField(entry.inlineIndex, 'title', ($event.target as HTMLInputElement).value)"
                />
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground md:col-span-2">
                {{ $st('Instructions') }}
                <textarea
                  :value="entry.fragment?.body || ''"
                  :disabled="editingDisabled"
                  rows="6"
                  spellcheck="false"
                  class="w-full resize-y rounded-md border border-input bg-transparent px-2.5 py-1.5 text-sm leading-relaxed text-foreground outline-none focus:border-ring"
                  @input="updateFragmentField(entry.inlineIndex, 'body', ($event.target as HTMLTextAreaElement).value)"
                />
                <span class="text-[11px]">{{
                  $st('{bytes} bytes', { bytes: promptFragmentBodyBytes(entry.fragment?.body || '') })
                }}</span>
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('Model patterns') }}
                <input
                  :value="entry.fragment?.when.models.join(', ') || ''"
                  :disabled="editingDisabled"
                  spellcheck="false"
                  class="rounded-md border border-input bg-transparent px-2.5 py-1.5 font-mono text-xs text-foreground outline-none focus:border-ring"
                  @change="updateFragmentModels(entry.inlineIndex, ($event.target as HTMLInputElement).value)"
                />
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
                {{ $st('Required tools') }}
                <select
                  :value="
                    entry.fragment?.when.requires_tools === null ? '' : String(entry.fragment?.when.requires_tools)
                  "
                  :disabled="editingDisabled"
                  class="rounded-md border border-input bg-background px-2.5 py-1.5 text-sm text-foreground outline-none focus:border-ring"
                  @change="updateFragmentTools(entry.inlineIndex, ($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="option in requiresToolsOptions" :key="option.value" :value="option.value">
                    {{ option.label }}
                  </option>
                </select>
              </label>
              <div class="md:col-span-2">
                <Button variant="ghost" size="sm" :disabled="editingDisabled" @click="removeFragment(entry.id)">
                  <RiDeleteBinLine class="mr-1.5 h-4 w-4" />
                  {{ $st('Remove fragment') }}
                </Button>
              </div>
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <select
              v-model="orderedFragmentInput"
              :disabled="editingDisabled"
              class="min-w-0 flex-1 rounded-md border border-input bg-background px-2.5 py-1.5 text-sm outline-none focus:border-ring"
            >
              <option value="">{{ $st('Choose a saved prompt') }}</option>
              <option v-for="item in addable" :key="item.id" :value="item.id">
                {{ savedPromptDisplayName(item.fragment) }} · {{ item.fragment.category || item.scope }}
              </option>
            </select>
            <Button
              variant="outline"
              size="sm"
              :disabled="editingDisabled || !orderedFragmentInput.trim()"
              @click="addOrderedFragment"
            >
              {{ $st('Attach prompt') }}
            </Button>
          </div>
        </div>

        <div v-if="unorderedInline.length" class="grid min-w-0 gap-2">
          <h3 class="text-sm font-medium">{{ $st('Unordered inline fragments') }}</h3>
          <div
            v-for="fragment in unorderedInline"
            :key="fragment.id"
            class="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border/60 px-3 py-2"
          >
            <div class="grid min-w-0 gap-1">
              <span class="font-mono text-sm">{{ fragment.id }}</span>
              <span class="text-xs text-muted-foreground">{{ issueLabel('inline_not_in_order') }}</span>
            </div>
            <div class="flex items-center gap-1">
              <Button
                variant="outline"
                size="sm"
                :disabled="editingDisabled"
                @click="layer = promptLayerAddOrder(layer, fragment.id)"
              >
                {{ $st('Add to order') }}
              </Button>
              <Button variant="ghost" size="sm" :disabled="editingDisabled" @click="removeFragment(fragment.id)">
                <RiDeleteBinLine class="h-4 w-4" />
                <span class="sr-only">{{ $st('Remove') }}</span>
              </Button>
            </div>
          </div>
        </div>

        <div class="grid min-w-0 gap-2">
          <h3 class="text-sm font-medium">{{ $st('Disabled inherited fragments') }}</h3>
          <p v-if="!layer.disabled.length" class="text-sm text-muted-foreground">
            {{ $st('No inherited fragments are disabled.') }}
          </p>
          <div class="flex flex-wrap gap-2">
            <span
              v-for="id in layer.disabled"
              :key="id"
              class="inline-flex items-center gap-2 rounded-full border border-border/70 px-2.5 py-1 text-xs"
            >
              <span>{{ availableById.get(id)?.fragment.title || id }}</span>
              <button
                type="button"
                class="text-muted-foreground hover:text-foreground"
                :disabled="editingDisabled"
                @click="enableFragment(id)"
              >
                {{ $st('Enable') }}
              </button>
            </span>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <select
              v-model="disabledFragmentInput"
              :disabled="editingDisabled"
              class="min-w-0 flex-1 rounded-md border border-input bg-background px-2.5 py-1.5 text-sm outline-none focus:border-ring"
            >
              <option value="">{{ $st('Choose an inherited prompt') }}</option>
              <option v-for="item in hideable" :key="item.id" :value="item.id">
                {{ savedPromptDisplayName(item.fragment) }} · {{ item.scope }}
              </option>
            </select>
            <Button
              variant="outline"
              size="sm"
              :disabled="editingDisabled || !disabledFragmentInput.trim()"
              @click="addDisabledFragment"
            >
              {{ $st('Hide inherited prompt') }}
            </Button>
          </div>
        </div>
      </div>
    </SettingsCard>

    <SettingsSaveBar
      v-if="scope !== 'effective' && contextAvailable && loaded"
      :dirty="dirty"
      :saving="saving"
      :disabled="editingDisabled"
      :save-disabled="!valid"
      @save="save"
      @discard="discard"
    />
  </div>
</template>
