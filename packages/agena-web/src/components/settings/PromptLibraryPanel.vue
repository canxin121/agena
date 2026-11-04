<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import { confirmAction } from '@/lib/appConfirm'
import {
  emptyPromptScopeConfig,
  normalizePromptFragment,
  normalizePromptScopeConfig,
  promptScopeConfigIsEmpty,
  type PromptFragment,
  type PromptScopeConfig,
} from '@/lib/promptLayers'
import {
  nextSavedPromptId,
  savedPromptDisplayName,
  savedPromptIsValid,
  savedPromptMatches,
  scopeUsesPrompt,
} from '@/lib/promptLibrary'
import { deleteRuntimeSetting, getRuntimeSettingLayer, setRuntimeSetting } from '@/lib/runtimeSettings'
import { settingsText as st } from '@/i18n/settingsText'
import { useDirectoryStore } from '@/stores/directory'

const props = defineProps<{ scope: string }>()
const directory = useDirectoryStore()
const source = computed<'global' | 'workspace'>(() => (props.scope === 'workspace' ? 'workspace' : 'global'))
const globalConfig = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const workspaceConfig = ref<PromptScopeConfig>(emptyPromptScopeConfig())
const baseline = computed(() => (source.value === 'global' ? globalConfig.value : workspaceConfig.value))
const selectedId = ref('')
const draft = ref<PromptFragment | null>(null)
const draftOrigin = ref<'library' | 'inline' | 'new'>('library')
const original = ref('')
const search = ref('')
const categoryFilter = ref('')
const loading = ref(false)
const saving = ref(false)
const loaded = ref(false)
const error = ref('')
let generation = 0

const entries = computed(() => [
  ...baseline.value.library.map((fragment) => ({ fragment, origin: 'library' as const })),
  ...baseline.value.inline.map((fragment) => ({ fragment, origin: 'inline' as const })),
])
const categories = computed(() =>
  [...new Set(entries.value.map(({ fragment }) => fragment.category?.trim()).filter((value): value is string => !!value))]
    .sort((a, b) => a.localeCompare(b)),
)
const filtered = computed(() =>
  entries.value.filter(({ fragment }) =>
    (!categoryFilter.value || (fragment.category || '') === categoryFilter.value) &&
    savedPromptMatches(fragment, search.value),
  ),
)
const dirty = computed(
  () => draft.value !== null && (draftOrigin.value === 'new' || JSON.stringify(draft.value) !== original.value),
)
const valid = computed(() => draft.value !== null && savedPromptIsValid(draft.value))
const used = computed(() => {
  const id = selectedId.value
  return !!id && (scopeUsesPrompt(globalConfig.value, id) || scopeUsesPrompt(workspaceConfig.value, id))
})

function beginEditing(fragment: PromptFragment, origin: 'library' | 'inline' | 'new') {
  draft.value = normalizePromptFragment(fragment)
  draftOrigin.value = origin
  original.value = JSON.stringify(draft.value)
  selectedId.value = fragment.id
  error.value = ''
}

async function canDiscard(): Promise<boolean> {
  if (saving.value) return false
  if (!dirty.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

async function load(preferId = selectedId.value) {
  const revision = ++generation
  loading.value = true
  loaded.value = false
  error.value = ''
  try {
    const [global, workspace] = await Promise.all([
      getRuntimeSettingLayer('global', 'prompts'),
      getRuntimeSettingLayer('workspace', 'prompts'),
    ])
    if (generation !== revision) return
    globalConfig.value = normalizePromptScopeConfig(global.value)
    workspaceConfig.value = normalizePromptScopeConfig(workspace.value)
    const config = source.value === 'global' ? globalConfig.value : workspaceConfig.value
    const items = [
      ...config.library.map((fragment) => ({ fragment, origin: 'library' as const })),
      ...config.inline.map((fragment) => ({ fragment, origin: 'inline' as const })),
    ]
    const chosen = items.find(({ fragment }) => fragment.id === preferId) || items[0]
    if (chosen) beginEditing(chosen.fragment, chosen.origin)
    else {
      draft.value = null
      selectedId.value = ''
      original.value = ''
      draftOrigin.value = 'library'
    }
    loaded.value = true
  } catch (reason) {
    if (generation === revision) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (generation === revision) loading.value = false
  }
}

async function selectPrompt(id: string) {
  if (selectedId.value === id || !(await canDiscard())) return
  const entry = entries.value.find(({ fragment }) => fragment.id === id)
  if (entry) beginEditing(entry.fragment, entry.origin)
}

async function addPrompt() {
  if (!(await canDiscard())) return
  const id = nextSavedPromptId(source.value, globalConfig.value, workspaceConfig.value)
  beginEditing(
    { id, title: '', category: '', body: '', when: { models: [], requires_tools: null } },
    'new',
  )
}

function updateField(field: 'title' | 'category' | 'body', value: string) {
  if (draft.value) draft.value = { ...draft.value, [field]: value }
}

function updateModels(value: string) {
  if (!draft.value) return
  draft.value = {
    ...draft.value,
    when: { ...draft.value.when, models: value.split(',').map((item) => item.trim()).filter(Boolean) },
  }
}

function updateTools(value: string) {
  if (!draft.value) return
  draft.value = {
    ...draft.value,
    when: { ...draft.value.when, requires_tools: value === '' ? null : value === 'true' },
  }
}

/** Merge into the current config, never overwrite its active layers/model overlays. */
async function persist(change: 'save' | 'delete') {
  if (!draft.value || saving.value || (change === 'save' && !valid.value)) return
  saving.value = true
  error.value = ''
  try {
    const selected = { ...draft.value, when: { ...draft.value.when } }
    const latest = normalizePromptScopeConfig((await getRuntimeSettingLayer(source.value, 'prompts')).value)
    if (
      JSON.stringify(latest.library) !== JSON.stringify(baseline.value.library) ||
      JSON.stringify(latest.inline) !== JSON.stringify(baseline.value.inline)
    ) {
      throw new Error(st('Saved prompts changed elsewhere. Refresh and try again.'))
    }
    const next: PromptScopeConfig = { ...latest }
    const field = draftOrigin.value === 'inline' ? 'inline' : 'library'
    if (change === 'delete' && draftOrigin.value === 'new') {
      draft.value = null
      selectedId.value = ''
      return
    }
    if (change === 'delete' && used.value) {
      throw new Error(st('Remove this prompt from active layers before deleting it.'))
    }
    const items = [...next[field]]
    const index = items.findIndex((item) => item.id === selected.id)
    if (change === 'delete') {
      if (index >= 0) items.splice(index, 1)
    } else if (index >= 0) items[index] = normalizePromptFragment(selected)
    else items.push(normalizePromptFragment(selected))
    next[field] = items
    if (promptScopeConfigIsEmpty(next)) {
      await deleteRuntimeSetting('prompts', undefined, source.value)
    } else {
      await setRuntimeSetting('prompts', next, undefined, source.value)
    }
    await load(change === 'delete' ? '' : selected.id)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}

async function removePrompt() {
  if (!draft.value || saving.value) return
  if (draftOrigin.value === 'new') {
    draft.value = null
    selectedId.value = ''
    return
  }
  if (used.value) {
    error.value = st('Remove this prompt from active layers before deleting it.')
    return
  }
  const confirmed = await confirmAction({
    title: st('Delete saved prompt?'),
    description: st('Other sessions may still reference this prompt. Deleting it could break their instructions.'),
    confirmText: st('Delete'),
    cancelText: st('Cancel'),
  })
  if (confirmed) await persist('delete')
}

async function refresh() {
  if (await canDiscard()) await load()
}
defineExpose({ refresh })

watch(source, () => { void load('') })
watch(
  () => directory.currentDirectory,
  () => {
    if (source.value === 'workspace') void load('')
  },
)
onMounted(() => { void load('') })
onBeforeUnmount(() => { generation += 1 })
onBeforeRouteUpdate(async (to, from) => {
  if (to.path === from.path && to.query.view === from.query.view) return canDiscard()
  return canDiscard()
})
onBeforeRouteLeave(canDiscard)
</script>

<template>
  <div class="grid min-w-0 gap-4" :aria-busy="loading || saving">
    <SettingsCard :title="$st('Saved prompt library')" :description="$st('Create, organize, and reuse prompts without managing fragment IDs.')">
      <div v-if="error" class="mb-3 rounded-md border border-destructive/40 p-3 text-xs text-destructive" role="alert">{{ error }}</div>
      <p v-if="loading" class="text-sm text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
      <div v-else-if="loaded" class="grid min-w-0 gap-4 md:grid-cols-[minmax(12rem,0.8fr)_minmax(0,1.6fr)]">
        <div class="grid min-w-0 content-start gap-3">
          <div class="flex items-center justify-between gap-2">
            <span class="text-sm font-medium">{{ $st('Saved prompts') }} ({{ entries.length }})</span>
            <div class="flex gap-1">
              <Button size="sm" variant="ghost" :disabled="saving" @click="refresh">
                <RiRefreshLine class="h-4 w-4" /><span class="sr-only">{{ $st('Refresh') }}</span>
              </Button>
              <Button size="sm" variant="outline" :disabled="saving" @click="addPrompt">
                <RiAddLine class="mr-1 h-4 w-4" />{{ $st('Add') }}
              </Button>
            </div>
          </div>
          <input
            v-model="search"
            :placeholder="$st('Search saved prompts')"
            class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm"
          />
          <select v-model="categoryFilter" class="min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm">
            <option value="">{{ $st('All categories') }}</option>
            <option v-for="category in categories" :key="category" :value="category">{{ category }}</option>
          </select>
          <div class="grid max-h-[35rem] min-w-0 gap-1 overflow-auto">
            <button
              v-for="entry in filtered"
              :key="entry.fragment.id"
              type="button"
              class="min-w-0 rounded-md border px-3 py-2 text-left hover:bg-muted/50"
              :class="selectedId === entry.fragment.id ? 'border-primary/50 bg-primary/10' : 'border-border/60'"
              @click="selectPrompt(entry.fragment.id)"
            >
              <span class="block truncate text-sm font-medium">{{ savedPromptDisplayName(entry.fragment) }}</span>
              <span class="block truncate text-xs text-muted-foreground">
                {{ entry.fragment.category || $st('Uncategorized') }}
                <template v-if="entry.origin === 'inline'"> · {{ $st('Legacy inline') }}</template>
              </span>
            </button>
            <p v-if="!filtered.length" class="p-2 text-sm text-muted-foreground">{{ $st('No saved prompts found.') }}</p>
          </div>
        </div>
        <div v-if="draft" class="grid min-w-0 content-start gap-3">
          <div class="flex items-center justify-between gap-2">
            <h3 class="text-sm font-medium">{{ draftOrigin === 'new' ? $st('New prompt') : $st('Edit prompt') }}</h3>
            <span v-if="used" class="text-xs text-muted-foreground">{{ $st('Used in active layers') }}</span>
          </div>
          <label class="grid gap-1 text-xs text-muted-foreground">
            {{ $st('Title') }}
            <input :value="draft.title" :disabled="saving" class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              @input="updateField('title', ($event.target as HTMLInputElement).value)" />
          </label>
          <label class="grid gap-1 text-xs text-muted-foreground">
            {{ $st('Category') }}
            <input :value="draft.category || ''" :disabled="saving" :placeholder="$st('Any category, e.g. writing, coding, research')"
              class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              @input="updateField('category', ($event.target as HTMLInputElement).value)" />
          </label>
          <label class="grid gap-1 text-xs text-muted-foreground">
            {{ $st('Instructions') }}
            <textarea :value="draft.body" :disabled="saving" rows="12"
              class="min-h-40 w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
              @input="updateField('body', ($event.target as HTMLTextAreaElement).value)" />
          </label>
          <details class="rounded-md border border-border/60 p-3">
            <summary class="cursor-pointer text-sm">{{ $st('Conditions') }}</summary>
            <div class="mt-3 grid gap-3">
              <label class="grid gap-1 text-xs text-muted-foreground">
                {{ $st('Model patterns') }}
                <input :value="draft.when.models.join(', ')" :disabled="saving" spellcheck="false"
                  class="rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                  @change="updateModels(($event.target as HTMLInputElement).value)" />
              </label>
              <label class="grid gap-1 text-xs text-muted-foreground">
                {{ $st('Required tools') }}
                <select :value="draft.when.requires_tools === null ? '' : String(draft.when.requires_tools)"
                  :disabled="saving" class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  @change="updateTools(($event.target as HTMLSelectElement).value)">
                  <option value="">{{ $st('Inherit') }}</option>
                  <option value="true">{{ $st('Required') }}</option>
                  <option value="false">{{ $st('Not required') }}</option>
                </select>
              </label>
            </div>
          </details>
          <p v-if="draftOrigin === 'inline'" class="text-xs text-muted-foreground">
            {{ $st('Legacy inline prompts remain attached to their original layers.') }}
          </p>
          <div class="flex flex-wrap items-center justify-between gap-3">
            <Button variant="ghost" color="danger" :disabled="saving" @click="removePrompt">
              <RiDeleteBinLine class="mr-1 h-4 w-4" />{{ $st('Delete') }}
            </Button>
            <Button :disabled="saving || !valid || !dirty" @click="persist('save')">
              {{ saving ? $st('Saving…') : $st('Save changes') }}
            </Button>
          </div>
        </div>
        <p v-else class="text-sm text-muted-foreground">{{ $st('Select a saved prompt or add a new one.') }}</p>
      </div>
    </SettingsCard>
  </div>
</template>
