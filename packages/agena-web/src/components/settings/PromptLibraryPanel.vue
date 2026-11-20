<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction } from '@/lib/appConfirm'
import { listBots } from '@/lib/bots'
import {
  MAX_PROMPT_TOTAL_BYTES,
  normalizePromptFragment,
  normalizePromptScopeConfig,
  promptFragmentBodyBytes,
  type PromptFragment,
  type PromptScopeConfig,
} from '@/lib/promptLayers'
import {
  availableSavedPrompts,
  changePromptCategory,
  composedPromptText,
  nextSavedPromptId,
  promptCategories,
  savedPromptDisplayName,
  savedPromptIsValid,
  savedPromptMatches,
  scopeUsesPrompt,
} from '@/lib/promptLibrary'
import { readRuntimeSettingSources, setRuntimeSetting, type RuntimeSettingsReadBundle } from '@/lib/runtimeSettings'
import { useDirectoryStore } from '@/stores/directory'
import PromptReferenceList from './PromptReferenceList.vue'

const props = defineProps<{ scope: 'global' | 'workspace' }>()
const directory = useDirectoryStore()
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const botReferences = ref<string[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const search = ref('')
const filter = ref<string | null>('')
const category = ref('')
const categoryName = ref('')
const selected = ref('')
const draft = ref<PromptFragment | null>(null)
const origin = ref<'library' | 'inline' | 'new'>('new')
const composing = ref(false)
const original = ref('')
let generation = 0

const global = computed(() => normalizePromptScopeConfig(sources.value?.global.value))
const workspace = computed(() => normalizePromptScopeConfig(sources.value?.workspace.value))
const current = computed(() => (props.scope === 'global' ? global.value : workspace.value))
const options = computed(() =>
  availableSavedPrompts(global.value, workspace.value, props.scope).map(({ fragment }) => fragment),
)
const entries = computed(() =>
  [...current.value.library, ...current.value.inline].sort((a, b) =>
    savedPromptDisplayName(a).localeCompare(savedPromptDisplayName(b)),
  ),
)
const categories = computed(() => promptCategories(current.value))
const filtered = computed(() =>
  entries.value.filter(
    (prompt) =>
      (filter.value === null ? !prompt.category : !filter.value || filter.value === prompt.category) &&
      savedPromptMatches(prompt, search.value),
  ),
)
const components = computed(() => options.value.filter((prompt) => prompt.id !== selected.value))
const preview = computed(() =>
  draft.value && composing.value
    ? composedPromptText(draft.value, new Map(components.value.map((prompt) => [prompt.id, prompt])))
    : null,
)
const exceedsBudget = computed(
  () => preview.value !== null && promptFragmentBodyBytes(preview.value) > MAX_PROMPT_TOTAL_BYTES,
)
const valid = computed(
  () =>
    !!draft.value &&
    savedPromptIsValid(draft.value) &&
    (!composing.value || (draft.value.parts?.length && preview.value !== null && !exceedsBudget.value)),
)
const dirty = computed(() => !!draft.value && JSON.stringify(draft.value) !== original.value)
const categoryDirty = computed(() => categoryName.value !== category.value)
const busy = computed(() => loading.value || saving.value)
const uses = computed(
  () =>
    scopeUsesPrompt(current.value, selected.value) ||
    (props.scope === 'global' &&
      (scopeUsesPrompt(workspace.value, selected.value) || botReferences.value.includes(selected.value))),
)

function show(prompt?: PromptFragment) {
  selected.value = prompt?.id ?? ''
  draft.value = prompt ? normalizePromptFragment(prompt) : null
  original.value = JSON.stringify(draft.value)
  origin.value = current.value.inline.some((item) => item.id === selected.value) ? 'inline' : 'library'
  composing.value = Boolean(prompt?.parts?.length)
}

function selectCategory(name: string) {
  category.value = name
  categoryName.value = name
}

async function canDiscard(includeCategory = true) {
  if (saving.value) return false
  if (!dirty.value && (!includeCategory || !categoryDirty.value)) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

async function load(prefer = selected.value) {
  const token = ++generation
  loading.value = true
  error.value = ''
  try {
    const [data, bots] = await Promise.all([
      readRuntimeSettingSources('prompts'),
      props.scope === 'global' ? listBots().catch(() => null) : null,
    ])
    if (token !== generation) return
    sources.value = data
    botReferences.value = bots?.items.flatMap((bot) => bot.persona.prompts?.ids ?? []) ?? []
    show(entries.value.find((prompt) => prompt.id === prefer) ?? entries.value[0])
    if (filter.value && !categories.value.includes(filter.value)) filter.value = ''
    selectCategory(categories.value.includes(category.value) ? category.value : '')
  } catch (reason) {
    if (token === generation) {
      sources.value = null
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (token === generation) loading.value = false
  }
}

async function choose(prompt: PromptFragment) {
  if (!(await canDiscard(false))) return
  error.value = ''
  show(prompt)
}

async function create(composition: boolean) {
  if (!(await canDiscard(false))) return
  show(
    normalizePromptFragment({
      id: nextSavedPromptId(props.scope, global.value, workspace.value),
      category: filter.value,
    }),
  )
  origin.value = 'new'
  composing.value = composition
  error.value = ''
}

async function persist(next: PromptScopeConfig, prefer: string) {
  const token = generation
  const scope = props.scope
  const base = sources.value?.[scope]
  if (!base || busy.value) return false
  saving.value = true
  error.value = ''
  try {
    if (base.revision === undefined)
      throw new Error(st('The configuration revision is missing. Refresh and try again.'))
    const response = await setRuntimeSetting('prompts', next, { expected_revision: base.revision }, scope)
    if (token !== generation || !sources.value) return false
    sources.value = {
      ...sources.value,
      [scope]: { ...base, value: response.current ?? next, revision: response.after_revision },
    }
    show(entries.value.find((prompt) => prompt.id === prefer))
    return true
  } catch (reason) {
    if (token === generation) error.value = reason instanceof Error ? reason.message : String(reason)
    return false
  } finally {
    saving.value = false
  }
}

async function savePrompt() {
  if (!draft.value || !valid.value) return
  const field = origin.value === 'inline' ? 'inline' : 'library'
  const entry = normalizePromptFragment({ ...draft.value, title: draft.value.title.trim() })
  const list = current.value[field]
  await persist(
    {
      ...current.value,
      [field]:
        origin.value === 'new' ? [...list, entry] : list.map((prompt) => (prompt.id === entry.id ? entry : prompt)),
    },
    entry.id,
  )
}

async function deletePrompt() {
  if (origin.value === 'new') return show()
  if (!draft.value || uses.value || busy.value) return
  const id = selected.value
  const token = generation
  const confirmed = await confirmAction({
    title: st('Delete saved prompt?'),
    confirmText: st('Delete'),
    cancelText: st('Cancel'),
  })
  if (!confirmed || token !== generation || id !== selected.value) return
  if (props.scope === 'global') {
    try {
      const bots = await listBots()
      if (token !== generation || id !== selected.value) return
      botReferences.value = bots.items.flatMap((bot) => bot.persona.prompts?.ids ?? [])
    } catch (reason) {
      if (token === generation) error.value = reason instanceof Error ? reason.message : String(reason)
      return
    }
    if (uses.value) return
  }
  const field = origin.value === 'inline' ? 'inline' : 'library'
  await persist({ ...current.value, [field]: current.value[field].filter((prompt) => prompt.id !== id) }, '')
}

async function saveCategory(remove = false) {
  const previous = category.value
  const name = remove ? '' : categoryName.value.trim()
  const token = generation
  if (remove ? !previous : !name || name === previous) return
  if (name && categories.value.includes(name)) {
    error.value = st('A category with this name already exists.')
    return
  }
  if (!(await canDiscard(false)) || token !== generation) return
  if (
    remove &&
    !(await confirmAction({
      title: st('Delete prompt category?'),
      description: st('Prompts in this category will become uncategorized.'),
      confirmText: st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  if (token !== generation || previous !== category.value) return
  if (
    await persist(changePromptCategory(current.value, previous, name), origin.value === 'new' ? '' : selected.value)
  ) {
    if (filter.value === previous) filter.value = name
    selectCategory(name)
  }
}

async function refresh() {
  if (await canDiscard()) await load()
}

defineExpose({ refresh, canDiscard, dirty, categoryDirty, saving })
watch(
  () => [props.scope, props.scope === 'workspace' ? directory.currentDirectory : null],
  () => {
    void load('')
  },
)
onMounted(() => {
  void load('')
})
onBeforeUnmount(() => {
  generation++
})
</script>

<template>
  <section class="settings-stack min-w-0" :aria-busy="busy" data-prompt-library>
    <div class="settings-toolbar rounded-lg border border-border/70 bg-card/30 p-2.5">
      <SearchInput
        v-model="search"
        class="settings-toolbar-search"
        :placeholder="$st('Search prompts…')"
        :input-aria-label="$st('Search prompts…')"
      />
      <select
        v-model="filter"
        :aria-label="$st('Prompt category')"
        class="min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-xs"
      >
        <option value="">{{ $st('All categories') }}</option>
        <option :value="null">{{ $st('Uncategorized') }}</option>
        <option v-for="name in categories" :key="name" :value="name">{{ name }}</option>
      </select>
      <IconButton
        size="sm"
        :loading="loading"
        :disabled="saving"
        :aria-label="$st('Refresh')"
        :tooltip="$st('Refresh')"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" aria-hidden="true" />
      </IconButton>
      <Button type="button" size="sm" variant="outline" :disabled="busy || !sources" @click="create(true)">{{
        $st('New composition')
      }}</Button>
      <Button type="button" size="sm" :disabled="busy || !sources" @click="create(false)">
        <RiAddLine class="me-1.5 h-4 w-4" aria-hidden="true" />{{ $st('New fragment') }}
      </Button>
    </div>
    <p
      v-if="error"
      role="alert"
      class="break-words rounded-md border border-destructive/30 p-2 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <details v-if="sources" class="rounded-md border border-border/70 bg-card/30 px-2.5 py-2">
      <summary class="cursor-pointer text-xs font-medium">{{ $st('Manage categories') }}</summary>
      <form class="settings-toolbar mt-2" @submit.prevent="saveCategory()">
        <select
          :value="category"
          class="min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-xs"
          :aria-label="$st('Prompt category')"
          :disabled="busy"
          @change="selectCategory(($event.target as HTMLSelectElement).value)"
        >
          <option value="">{{ $st('New category') }}</option>
          <option v-for="name in categories" :key="name" :value="name">{{ name }}</option>
        </select>
        <Input
          v-model="categoryName"
          class="h-8 min-w-0 flex-1"
          :aria-label="$st('Category name')"
          :placeholder="$st('Category name')"
          :disabled="busy"
        />
        <Button type="submit" size="sm" :disabled="busy || !categoryDirty || !categoryName.trim()">{{
          $st('Save')
        }}</Button>
        <IconButton
          v-if="category"
          type="button"
          size="sm"
          :disabled="busy"
          :aria-label="$st('Delete prompt category?')"
          :tooltip="$st('Delete')"
          @click="saveCategory(true)"
        >
          <RiDeleteBinLine class="h-4 w-4 text-destructive" aria-hidden="true" />
        </IconButton>
      </form>
    </details>
    <div v-if="sources" class="settings-master-detail">
      <div class="min-w-0 overflow-hidden rounded-md border border-border/70">
        <button
          v-for="prompt in filtered"
          :key="prompt.id"
          type="button"
          :disabled="busy"
          :aria-pressed="prompt.id === selected"
          class="flex w-full min-w-0 flex-wrap items-center justify-between gap-x-2 border-b border-border/50 px-2.5 py-1.5 text-start text-sm last:border-0 hover:bg-muted"
          :class="{ 'bg-muted': prompt.id === selected }"
          @click="choose(prompt)"
        >
          <span class="min-w-0 break-words font-medium">{{ savedPromptDisplayName(prompt) }}</span>
          <span class="break-words text-xs text-muted-foreground">{{
            [prompt.category, prompt.parts?.length ? $st('Prompt composition') : ''].filter(Boolean).join(' · ')
          }}</span>
        </button>
        <p v-if="!filtered.length" class="p-2.5 text-xs text-muted-foreground">{{ $st('No saved prompts') }}</p>
      </div>
      <form v-if="draft" class="min-w-0 rounded-md border border-border/70 p-2.5" @submit.prevent="savePrompt">
        <fieldset class="grid min-w-0 gap-2.5" :disabled="busy">
          <div class="settings-control-grid">
            <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
              {{ $st('Title') }}
              <Input v-model="draft.title" class="h-8" required />
            </label>
            <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
              {{ $st('Prompt category') }}
              <select
                v-model="draft.category"
                class="h-8 min-w-0 rounded-md border border-input bg-background px-2 text-xs text-foreground"
              >
                <option :value="undefined">{{ $st('Uncategorized') }}</option>
                <option v-for="name in categories" :key="name" :value="name">{{ name }}</option>
              </select>
            </label>
          </div>
          <template v-if="composing">
            <PromptReferenceList
              :model-value="draft.parts ?? []"
              :options="components"
              :disabled="busy"
              @update:model-value="draft.parts = $event"
            />
            <p v-if="draft.parts?.length && preview === null" role="alert" class="text-xs text-destructive">
              {{ $st('Check for missing or circular prompt references.') }}
            </p>
            <p v-if="exceedsBudget" role="alert" class="text-xs text-destructive">
              {{ $st('The combined prompt exceeds the size limit.') }}
            </p>
            <details v-if="preview" class="rounded-md border border-border/60 px-2.5 py-2">
              <summary class="cursor-pointer text-xs font-medium">{{ $st('Preview') }}</summary>
              <pre class="mt-2 max-h-64 overflow-y-auto whitespace-pre-wrap break-words text-xs">{{ preview }}</pre>
            </details>
          </template>
          <label v-else class="grid gap-1 text-xs text-muted-foreground">
            {{ $st('Instructions') }}
            <textarea
              v-model="draft.body"
              rows="7"
              spellcheck="false"
              required
              class="min-w-0 resize-y rounded-md border border-input bg-background p-2 text-sm text-foreground"
            />
          </label>
          <div class="settings-toolbar">
            <Button type="submit" size="sm" :disabled="!valid || (!dirty && origin !== 'new')">{{
              $st('Save')
            }}</Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              :disabled="origin !== 'new' && uses"
              @click="deletePrompt"
              >{{ origin === 'new' ? $st('Cancel') : $st('Delete') }}</Button
            >
            <span v-if="uses" class="text-xs text-muted-foreground">{{
              $st('Referenced by other prompts or active layers')
            }}</span>
          </div>
        </fieldset>
      </form>
    </div>
    <p v-else-if="loading" role="status" class="text-xs text-muted-foreground">{{ $st('Loading…') }}</p>
  </section>
</template>
