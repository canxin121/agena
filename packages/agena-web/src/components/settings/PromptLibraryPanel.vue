<script setup lang="ts">
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import {
  RiAddLine,
  RiDeleteBinLine,
  RiEditLine,
  RiFileTextLine,
  RiFolderAddLine,
  RiFolderLine,
  RiLock2Line,
  RiRefreshLine,
  RiStackLine,
} from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction } from '@/lib/appConfirm'
import { listBots } from '@/lib/bots'
import {
  MAX_PROMPT_FRAGMENT_BYTES,
  MAX_PROMPT_TOTAL_BYTES,
  normalizePromptFragment,
  normalizePromptScopeConfig,
  promptFragmentBodyBytes,
  type BuiltinPrompt,
  type PromptFragment,
  type PromptScopeConfig,
} from '@/lib/promptLayers'
import {
  availableSavedPrompts,
  changePromptCategory,
  composedPromptText,
  groupSavedPrompts,
  nextSavedPromptId,
  promptExcerpt,
  savedPromptDisplayName,
  savedPromptExcerpt,
  savedPromptIsValid,
  savedPromptMatches,
  scopeUsesPrompt,
} from '@/lib/promptLibrary'
import { readRuntimeSettingSources, setRuntimeSetting, type RuntimeSettingsReadBundle } from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'
import PromptReferenceList from './PromptReferenceList.vue'
import {
  builtinPromptApplicability,
  builtinPromptTitle,
  readBuiltinPrompts,
  reusableBuiltinPrompts,
} from './promptCatalog'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SettingsListDetail from './workbench/SettingsListDetail.vue'

const props = defineProps<{ scope: 'global' | 'workspace' }>()
const emit = defineEmits<{ 'workspace-config-path': [path: string] }>()
const toasts = useToastsStore()
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const botReferences = ref<string[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const search = ref('')
const filter = ref<string | null>('')
const builtinOnly = ref(false)
const builtins = ref<BuiltinPrompt[]>([])
const builtinSelection = ref<BuiltinPrompt | null>(null)
const categoriesOpen = ref(false)
const category = ref('')
const categoryName = ref('')
const categoryError = ref('')
const draft = ref<PromptFragment | null>(null)
const composing = ref(false)
const showDetail = ref(false)
const listDetail = ref<InstanceType<typeof SettingsListDetail> | null>(null)
const editorScroll = ref<HTMLElement | null>(null)
let generation = 0
let loadController: AbortController | undefined

const global = computed(() => normalizePromptScopeConfig(sources.value?.global.value))
const workspace = computed(() => normalizePromptScopeConfig(sources.value?.workspace.value))
const current = computed(() => (props.scope === 'global' ? global.value : workspace.value))
const options = computed(() =>
  availableSavedPrompts(global.value, workspace.value, props.scope, reusableBuiltinPrompts(builtins.value)).map(
    ({ fragment }) => fragment,
  ),
)
const optionsById = computed(() => new Map(options.value.map((prompt) => [prompt.id, prompt])))
const groups = computed(() => groupSavedPrompts(current.value))
const entries = computed(() =>
  groups.value
    .flatMap(({ prompts }) => prompts)
    .sort((left, right) => savedPromptDisplayName(left).localeCompare(savedPromptDisplayName(right))),
)
const categoryOptions = computed(() =>
  groups.value.flatMap(({ category }) => (category === null ? [] : [{ value: category, label: category }])),
)
const selected = computed(() => builtinSelection.value?.id || draft.value?.id || '')
const saved = computed(() => entries.value.find((prompt) => prompt.id === selected.value))
const isNew = computed(() => Boolean(draft.value && !saved.value))
const field = computed(() =>
  current.value.inline.some((prompt) => prompt.id === selected.value) ? 'inline' : 'library',
)
const editorTitle = computed(() =>
  saved.value ? savedPromptDisplayName(saved.value) : composing.value ? st('New composition') : st('New fragment'),
)
const activeCategory = computed(() => groups.value.find(({ category }) => category === filter.value))
const hasSearch = computed(() => Boolean(search.value.trim()))
const visiblePrompts = computed(() => {
  if (builtinOnly.value) return []
  const prompts = filter.value === '' ? entries.value : (activeCategory.value?.prompts ?? [])
  return hasSearch.value ? prompts.filter((prompt) => savedPromptMatches(prompt, search.value)) : prompts
})
const visibleBuiltins = computed(() =>
  filter.value === ''
    ? builtins.value.filter((prompt) => {
        const needle = search.value.trim().toLocaleLowerCase()
        return [prompt.id, builtinPromptTitle(prompt), prompt.body].some((part) =>
          part.toLocaleLowerCase().includes(needle),
        )
      })
    : [],
)
const components = computed(() => options.value.filter((prompt) => prompt.id !== selected.value))
const preview = computed(() =>
  draft.value && composing.value
    ? composedPromptText(draft.value, new Map(components.value.map((prompt) => [prompt.id, prompt])))
    : null,
)
const exceedsCompositionBudget = computed(
  () => preview.value !== null && promptFragmentBodyBytes(preview.value) > MAX_PROMPT_TOTAL_BYTES,
)
const exceedsFragmentBudget = computed(
  () => !!draft.value && promptFragmentBodyBytes(draft.value.body) > MAX_PROMPT_FRAGMENT_BYTES,
)
const valid = computed(
  () =>
    !!draft.value &&
    savedPromptIsValid(draft.value) &&
    (!composing.value || (draft.value.parts?.length && preview.value !== null && !exceedsCompositionBudget.value)),
)
const dirty = computed(
  () =>
    !!draft.value &&
    (isNew.value || JSON.stringify(normalizePromptFragment(draft.value)) !== JSON.stringify(saved.value)),
)
const categoryDirty = computed(() => categoriesOpen.value && categoryName.value.trim() !== category.value)
const busy = computed(() => loading.value || saving.value)
const uses = computed(
  () =>
    Boolean(saved.value) &&
    (scopeUsesPrompt(current.value, selected.value) ||
      (props.scope === 'global' &&
        (scopeUsesPrompt(workspace.value, selected.value) || botReferences.value.includes(selected.value)))),
)

function show(prompt?: PromptFragment, composition = Boolean(prompt?.parts?.length)) {
  builtinSelection.value = null
  draft.value = prompt ? normalizePromptFragment(prompt) : null
  composing.value = composition
  if (!prompt) showDetail.value = false
}

async function openEditor() {
  showDetail.value = true
  await nextTick()
  editorScroll.value?.scrollTo({ top: 0 })
  listDetail.value?.focusDetail()
}

async function backToList() {
  showDetail.value = false
  await nextTick()
  listDetail.value?.focusList()
}

function clearFilters() {
  builtinOnly.value = false
  search.value = ''
  filter.value = ''
}

function resetMissingCategoryFilter() {
  if (filter.value !== '' && !activeCategory.value) filter.value = ''
}

function selectCategory(name: string) {
  category.value = name
  categoryName.value = name
  categoryError.value = ''
}

function openCategory(name: string) {
  selectCategory(name)
  categoriesOpen.value = true
}

function closeCategories() {
  if (saving.value) return
  categoriesOpen.value = false
  selectCategory('')
}

const { canDiscard } = useSettingsDraftGuard({
  dirty: () => dirty.value || categoryDirty.value,
  busy: () => saving.value,
  dialog: () => ({ description: st('You have unsaved changes.') }),
  sameTarget: (to, from) =>
    to.path === from.path && to.query.view === from.query.view && to.query.scope === from.query.scope,
})

async function load(prefer = selected.value) {
  const token = ++generation
  loadController?.abort()
  const controller = new AbortController()
  loadController = controller
  loading.value = true
  error.value = ''
  try {
    const [data, bots, catalog] = await Promise.all([
      readRuntimeSettingSources('prompts', controller.signal),
      props.scope === 'global' ? listBots(controller.signal).catch(() => null) : null,
      readBuiltinPrompts(controller.signal),
    ])
    if (token !== generation) return
    sources.value = data
    emit('workspace-config-path', data.workspace.config_path || '')
    botReferences.value = bots?.items.flatMap((bot) => bot.persona.prompts?.ids ?? []) ?? []
    builtins.value = catalog
    show(entries.value.find((prompt) => prompt.id === prefer) ?? entries.value[0])
    builtinSelection.value =
      catalog.find((prompt) => prompt.id === prefer) ?? (draft.value ? null : (catalog[0] ?? null))
    if (builtinSelection.value) draft.value = null
    resetMissingCategoryFilter()
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
  if (busy.value) return
  if (selected.value === prompt.id) return openEditor()
  const token = generation
  if (!(await canDiscard()) || token !== generation || busy.value) return
  error.value = ''
  show(prompt)
  await openEditor()
}

async function chooseBuiltin(prompt: BuiltinPrompt) {
  if (busy.value) return
  const token = generation
  if (!(await canDiscard()) || token !== generation || busy.value) return
  error.value = ''
  show()
  builtinSelection.value = prompt
  await openEditor()
}

async function filterBuiltins() {
  const token = generation
  if (!(await canDiscard()) || token !== generation || busy.value) return
  filter.value = ''
  builtinOnly.value = true
  if (!builtinSelection.value && visibleBuiltins.value.length) {
    show()
    builtinSelection.value = visibleBuiltins.value[0] ?? null
  }
}

async function create(composition: boolean, promptCategory = filter.value) {
  const token = generation
  if (busy.value || !(await canDiscard()) || token !== generation || busy.value) return
  show(
    normalizePromptFragment({
      id: nextSavedPromptId(props.scope, global.value, workspace.value),
      category: promptCategory,
    }),
    composition,
  )
  error.value = ''
  await openEditor()
}

async function persist(next: PromptScopeConfig, errorTarget = error) {
  const token = generation
  const scope = props.scope
  const base = sources.value?.[scope]
  if (!base || busy.value) return false
  saving.value = true
  errorTarget.value = ''
  try {
    if (base.revision === undefined)
      throw new Error(st('The configuration revision is missing. Refresh and try again.'))
    const response = await setRuntimeSetting(
      'prompts',
      next,
      {
        expected_revision: base.revision,
        workspace_id: scope === 'workspace' && base.workspace_id != null ? String(base.workspace_id) : undefined,
      },
      scope,
    )
    if (token !== generation || !sources.value) return false
    sources.value = {
      ...sources.value,
      [scope]: { ...base, value: response.current ?? next, revision: response.after_revision },
    }
    return true
  } catch (reason) {
    if (token === generation) errorTarget.value = reason instanceof Error ? reason.message : String(reason)
    return false
  } finally {
    saving.value = false
  }
}

async function savePrompt() {
  if (busy.value || !draft.value || !dirty.value || !valid.value) return
  const entry = normalizePromptFragment({ ...draft.value, title: draft.value.title.trim() })
  const revealEntry = isNew.value || entry.category !== saved.value?.category
  const list = current.value[field.value]
  if (
    await persist({
      ...current.value,
      [field.value]: isNew.value ? [...list, entry] : list.map((prompt) => (prompt.id === entry.id ? entry : prompt)),
    })
  ) {
    builtinOnly.value = false
    if (revealEntry) {
      if (filter.value !== '') filter.value = entry.category ?? null
      if (!savedPromptMatches(entry, search.value)) search.value = ''
    }
    show(saved.value)
    toasts.push('success', st('Saved'))
  }
}

function discardPrompt() {
  if (busy.value) return
  error.value = ''
  if (!saved.value) void backToList()
  show(saved.value)
}

function saveFromKeyboard(event: KeyboardEvent) {
  if (event.isComposing || (!event.metaKey && !event.ctrlKey) || event.key.toLowerCase() !== 's') return
  event.preventDefault()
  void savePrompt()
}

async function deletePrompt() {
  if (!saved.value || uses.value || busy.value) return
  const id = selected.value
  const token = generation
  const confirmed = await confirmAction({
    title: st('Delete saved prompt?'),
    confirmText: st('Delete'),
    cancelText: st('Cancel'),
  })
  if (!confirmed || token !== generation || id !== selected.value || busy.value) return
  if (props.scope === 'global') {
    try {
      const bots = await listBots()
      if (token !== generation || id !== selected.value) return
      botReferences.value = bots.items.flatMap((bot) => bot.persona.prompts?.ids ?? [])
    } catch (reason) {
      if (token === generation) error.value = reason instanceof Error ? reason.message : String(reason)
      return
    }
    if (uses.value || busy.value) return
  }
  if (
    await persist({ ...current.value, [field.value]: current.value[field.value].filter((prompt) => prompt.id !== id) })
  ) {
    resetMissingCategoryFilter()
    show(visiblePrompts.value[0])
    if (!draft.value) await backToList()
  }
}

async function saveCategory(remove = false) {
  const previous = category.value
  const name = remove ? '' : categoryName.value.trim()
  const token = generation
  if (busy.value || (remove ? !previous : !name || name === previous)) return
  if (name && groups.value.some(({ category }) => category === name)) {
    categoryError.value = st('A category with this name already exists.')
    return
  }
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
  if (token !== generation || !categoriesOpen.value || previous !== category.value) return
  if (await persist(changePromptCategory(current.value, previous, name), categoryError)) {
    if (previous && draft.value?.category === previous) {
      draft.value = normalizePromptFragment({ ...draft.value, category: name })
    }
    if (!previous) {
      search.value = ''
      filter.value = name
    } else if (filter.value === previous) {
      filter.value = name || null
    }
    resetMissingCategoryFilter()
    closeCategories()
    toasts.push('success', st('Saved'))
  }
}

async function refresh() {
  const token = generation
  if (!busy.value && (await canDiscard()) && token === generation) {
    closeCategories()
    await load()
  }
}

defineExpose({ refresh })
watch([filter, search, builtinOnly], () => listDetail.value?.scrollListToTop(), { flush: 'post' })
watch(
  () => props.scope,
  () => {
    sources.value = null
    clearFilters()
    closeCategories()
    show()
    void load('')
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  generation++
  loadController?.abort()
})
</script>

<template>
  <section class="settings-viewport" :aria-busy="busy" data-prompt-library>
    <div class="settings-toolbar justify-between rounded-lg border border-border/70 bg-card/30 p-2.5">
      <Button size="sm" variant="ghost" :disabled="busy || !sources" @click="openCategory('')">
        <RiFolderAddLine class="me-1.5 h-4 w-4 shrink-0" aria-hidden="true" />{{ $st('New category') }}
      </Button>
      <div class="settings-toolbar">
        <IconButton
          size="sm"
          :loading="loading"
          :disabled="busy"
          :aria-label="$st('Refresh')"
          :tooltip="$st('Refresh')"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" aria-hidden="true" />
        </IconButton>
        <Button size="sm" variant="outline" :disabled="busy || !sources" @click="create(true)">
          <RiStackLine class="me-1.5 h-4 w-4 shrink-0" aria-hidden="true" />{{ $st('New composition') }}
        </Button>
        <Button size="sm" :disabled="busy || !sources" @click="create(false)">
          <RiAddLine class="me-1.5 h-4 w-4 shrink-0" aria-hidden="true" />{{ $st('New fragment') }}
        </Button>
      </div>
    </div>
    <p
      v-if="error"
      role="alert"
      class="break-words rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <SettingsListDetail
      v-if="loading || sources"
      ref="listDetail"
      v-model:detail-open="showDetail"
      :list-label="$st('Prompt library')"
      :detail-label="editorTitle"
      :back-label="$st('Prompts')"
      :list-busy="loading"
      :has-detail="Boolean(draft || builtinSelection || entries.length)"
      list-scroll-class="space-y-2"
    >
      <template #list-header>
        <div class="flex min-w-0 items-center justify-between gap-2">
          <h2 data-settings-list-heading tabindex="-1" class="min-w-0 px-0.5 text-sm font-semibold outline-none">
            {{ $st('Prompts') }}
          </h2>
          <button
            v-if="groups.length || builtins.length"
            type="button"
            :disabled="busy"
            :aria-pressed="filter === '' && !builtinOnly"
            :title="$st('All categories')"
            class="prompt-category-filter"
            data-prompt-category-filter=""
            @click="((filter = ''), (builtinOnly = false))"
          >
            <span class="truncate">{{ $st('All categories') }}</span>
            <span class="shrink-0 tabular-nums text-muted-foreground">{{ entries.length + builtins.length }}</span>
          </button>
        </div>
        <SearchInput
          v-model="search"
          :placeholder="$st('Search prompts…')"
          :input-aria-label="$st('Search prompts…')"
          :show-search-button="false"
        />
        <div
          v-if="groups.length || builtins.length"
          role="group"
          :aria-label="$st('Prompt category')"
          class="flex max-h-28 min-w-0 flex-wrap items-start gap-1.5 overflow-y-auto overscroll-contain p-0.5"
        >
          <button
            v-if="builtins.length"
            type="button"
            :disabled="busy"
            :aria-pressed="builtinOnly"
            class="prompt-category-filter"
            @click="filterBuiltins"
          >
            <RiLock2Line class="h-3.5 w-3.5 shrink-0 text-primary" aria-hidden="true" />
            <span class="truncate">{{ $st('Built-in prompts') }}</span>
            <span class="shrink-0 tabular-nums text-muted-foreground">{{ builtins.length }}</span>
          </button>
          <button
            v-for="group in groups"
            :key="group.category"
            type="button"
            :disabled="busy"
            :aria-pressed="filter === group.category"
            :title="group.category ?? $st('Uncategorized')"
            :data-prompt-category-filter="group.category"
            class="prompt-category-filter"
            @click="((filter = group.category), (builtinOnly = false))"
          >
            <span class="truncate">{{ group.category ?? $st('Uncategorized') }}</span>
            <span class="shrink-0 tabular-nums text-muted-foreground">{{ group.prompts.length }}</span>
          </button>
        </div>
      </template>
      <template #list>
        <header
          v-if="activeCategory"
          class="flex min-w-0 items-center gap-1 rounded-md border border-border/60 bg-muted/25 px-2 py-1"
          :data-prompt-category="activeCategory.category ?? ''"
        >
          <RiFolderLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <h3
            class="min-w-0 flex-1 truncate text-xs font-semibold"
            :title="activeCategory.category ?? $st('Uncategorized')"
          >
            {{ activeCategory.category ?? $st('Uncategorized') }}
          </h3>
          <IconButton
            size="sm"
            :disabled="busy"
            :aria-label="`${$st('New fragment')} · ${activeCategory.category ?? $st('Uncategorized')}`"
            :tooltip="$st('New fragment')"
            @click="create(false, activeCategory.category)"
          >
            <RiAddLine class="h-4 w-4" aria-hidden="true" />
          </IconButton>
          <IconButton
            v-if="activeCategory.category !== null"
            size="sm"
            :disabled="busy"
            :aria-label="`${$st('Edit')} · ${activeCategory.category}`"
            :tooltip="$st('Edit')"
            @click="openCategory(activeCategory.category)"
          >
            <RiEditLine class="h-4 w-4" aria-hidden="true" />
          </IconButton>
        </header>
        <p v-if="loading && !sources" role="status" class="px-3 py-8 text-center text-sm text-muted-foreground">
          {{ $st('Loading…') }}
        </p>
        <div v-else-if="!visiblePrompts.length && !visibleBuiltins.length" class="space-y-2 px-3 py-8 text-center">
          <p role="status" class="text-sm text-muted-foreground">
            {{ hasSearch ? $st('No matching prompts') : $st('No saved prompts') }}
          </p>
          <Button v-if="hasSearch" size="sm" variant="ghost" @click="clearFilters">{{ $st('Clear filters') }}</Button>
        </div>
        <ul v-if="visiblePrompts.length" class="space-y-0.5">
          <li v-for="prompt in visiblePrompts" :key="prompt.id" class="min-w-0">
            <button
              type="button"
              :disabled="busy"
              :aria-pressed="prompt.id === selected"
              :data-prompt-entry="prompt.id"
              :data-keyboard-item="prompt.id"
              :data-keyboard-label="savedPromptDisplayName(prompt)"
              :data-keyboard-selected="String(prompt.id === selected)"
              class="flex w-full min-w-0 items-start gap-2 rounded-md px-2 py-1.5 text-start transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :class="prompt.id === selected ? 'bg-primary/10 ring-1 ring-inset ring-primary/25' : 'hover:bg-muted/60'"
              @click="choose(prompt)"
            >
              <component
                :is="prompt.parts?.length ? RiStackLine : RiFileTextLine"
                class="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-medium">{{ savedPromptDisplayName(prompt) }}</span>
                <span class="mt-0.5 block truncate text-xs text-muted-foreground">
                  {{
                    savedPromptExcerpt(prompt, optionsById) ||
                    (prompt.parts?.length ? $st('Prompt composition') : prompt.category)
                  }}
                </span>
              </span>
            </button>
          </li>
        </ul>
        <section v-if="visibleBuiltins.length" class="space-y-1" data-prompt-builtin-list>
          <h3 class="flex items-center gap-2 px-2 pt-1 text-xs font-semibold text-muted-foreground">
            {{ $st('Built-in prompts') }}
          </h3>
          <ul class="space-y-0.5">
            <li v-for="prompt in visibleBuiltins" :key="prompt.id">
              <button
                type="button"
                :disabled="busy"
                :aria-pressed="prompt.id === selected"
                :data-prompt-builtin="prompt.id"
                :data-keyboard-item="prompt.id"
                :data-keyboard-label="builtinPromptTitle(prompt)"
                :data-keyboard-selected="String(prompt.id === selected)"
                class="flex w-full min-w-0 items-start gap-2 rounded-md px-2 py-1.5 text-start transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                :class="prompt.id === selected ? 'bg-primary/10 ring-1 ring-inset ring-primary/25' : 'hover:bg-muted/60'"
                @click="chooseBuiltin(prompt)"
              >
                <RiLock2Line class="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-sm font-medium">{{ builtinPromptTitle(prompt) }}</span>
                  <span class="mt-0.5 block truncate text-xs text-muted-foreground">
                    {{ promptExcerpt(prompt.body) }}
                  </span>
                </span>
              </button>
            </li>
          </ul>
        </section>
      </template>
      <template #detail>
        <form
          v-if="draft"
          class="flex min-h-0 min-w-0 flex-1 flex-col"
          :aria-label="editorTitle"
          data-prompt-editor
          @submit.prevent="savePrompt"
          @keydown="saveFromKeyboard"
        >
          <header class="flex shrink-0 items-center gap-2 rounded-t-lg border-b border-border/60 bg-muted/20 px-3 py-2">
            <component
              :is="composing ? RiStackLine : RiFileTextLine"
              class="h-4 w-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <h2
              data-settings-detail-heading
              tabindex="-1"
              class="min-w-0 flex-1 break-words text-sm font-semibold outline-none"
            >
              {{ editorTitle }}
            </h2>
            <IconButton
              v-if="saved"
              size="sm"
              :disabled="busy || uses"
              :aria-label="$st('Delete saved prompt?')"
              :tooltip="$st('Delete')"
              @click="deletePrompt"
            >
              <RiDeleteBinLine class="h-4 w-4 text-destructive" aria-hidden="true" />
            </IconButton>
          </header>
          <div ref="editorScroll" class="settings-viewport-scroll p-3 sm:p-4">
            <fieldset class="grid min-w-0 gap-4" :class="{ 'prompt-fragment-fields': !composing }" :disabled="busy">
              <div class="settings-control-grid">
                <label class="grid min-w-0 gap-1.5 text-xs font-medium text-muted-foreground">
                  {{ $st('Title') }}
                  <Input v-model="draft.title" class="bg-background font-normal text-foreground" required />
                </label>
                <div class="grid min-w-0 gap-1.5">
                  <span class="text-xs font-medium text-muted-foreground">{{ $st('Prompt category') }}</span>
                  <p
                    v-if="isNew"
                    class="flex min-h-9 min-w-0 items-center gap-2 rounded-md bg-muted/35 px-3 py-1.5 text-sm"
                    data-prompt-create-category
                  >
                    <RiFolderLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span class="min-w-0 [overflow-wrap:anywhere]">{{ draft.category ?? $st('Uncategorized') }}</span>
                  </p>
                  <OptionPicker
                    v-else
                    :model-value="draft.category ?? ''"
                    :options="categoryOptions"
                    :title="$st('Prompt category')"
                    :empty-label="$st('Uncategorized')"
                    :disabled="busy"
                    trigger-class="bg-background font-normal"
                    data-prompt-category-picker
                    @update:model-value="draft.category = $event || undefined"
                  />
                </div>
              </div>
              <div v-if="composing" class="grid min-w-0 gap-2.5">
                <PromptReferenceList
                  :model-value="draft.parts ?? []"
                  :options="components"
                  :disabled="busy"
                  @update:model-value="draft.parts = $event"
                />
                <p v-if="draft.parts?.length && preview === null" role="alert" class="text-xs text-destructive">
                  {{ $st('Check for missing or circular prompt references.') }}
                </p>
                <p v-if="exceedsCompositionBudget" role="alert" class="text-xs text-destructive">
                  {{ $st('The combined prompt exceeds the size limit.') }}
                </p>
                <section
                  v-if="preview"
                  class="min-w-0 rounded-md border border-border/60 bg-background"
                  data-prompt-preview
                >
                  <h3 class="border-b border-border/60 px-3 py-2 text-xs font-medium">
                    {{ $st('Preview') }}
                  </h3>
                  <pre class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words p-3 text-sm leading-relaxed">{{
                    preview
                  }}</pre>
                </section>
              </div>
              <label v-else class="prompt-body-field grid min-w-0 gap-1.5 text-xs font-medium text-muted-foreground">
                {{ $st('Instructions') }}
                <textarea
                  v-model="draft.body"
                  rows="12"
                  spellcheck="false"
                  required
                  :aria-invalid="exceedsFragmentBudget || undefined"
                  class="min-h-52 min-w-0 resize-y rounded-md border border-input bg-background px-3 py-2.5 text-sm font-normal leading-relaxed text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
                />
                <span v-if="exceedsFragmentBudget" role="alert" class="text-destructive">
                  {{ $st('The prompt fragment exceeds the size limit.') }}
                </span>
              </label>
              <p v-if="uses" class="text-xs text-muted-foreground">
                {{ $st('Referenced by other prompts or active layers') }}
              </p>
            </fieldset>
          </div>
          <SettingsSaveBar
            v-if="dirty || saving"
            class="m-2.5 shrink-0"
            :dirty="dirty"
            :saving="saving"
            :disabled="loading"
            :save-disabled="!valid"
            sticky
            data-prompt-save
            @discard="discardPrompt"
            @save="savePrompt"
          />
        </form>
        <article v-else-if="builtinSelection" class="flex min-h-0 min-w-0 flex-1 flex-col" data-prompt-builtin-detail>
          <header class="flex shrink-0 items-center gap-2.5 rounded-t-lg border-b border-border/60 bg-muted/20 px-3 py-2">
            <RiLock2Line class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <div class="min-w-0">
              <h2 data-settings-detail-heading tabindex="-1" class="break-words text-sm font-semibold outline-none">
                {{ builtinPromptTitle(builtinSelection) }}
              </h2>
              <p class="break-words text-xs text-muted-foreground">
                {{
                  builtinSelection.reusable
                    ? $st('Reusable built-in prompt')
                    : `${$st('Required built-in prompt')} · ${builtinPromptApplicability(builtinSelection.when)}`
                }}
              </p>
            </div>
          </header>
          <div ref="editorScroll" class="settings-viewport-scroll px-4 py-3 sm:px-5 sm:py-4">
            <pre class="whitespace-pre-wrap font-mono text-sm leading-relaxed [overflow-wrap:anywhere]">{{
              builtinSelection.body
            }}</pre>
          </div>
        </article>
        <div v-else-if="!loading && entries.length" class="settings-split-only">
          <div class="grid min-h-56 place-content-center gap-2 px-4 text-center text-sm text-muted-foreground">
            <RiFileTextLine class="mx-auto h-6 w-6" aria-hidden="true" />
            {{ $st('Select a saved prompt') }}
          </div>
        </div>
      </template>
    </SettingsListDetail>
    <FormDialog :open="categoriesOpen" :title="category || $st('New category')" @close="closeCategories">
      <form class="grid min-w-0 gap-3" data-prompt-categories @submit.prevent="saveCategory()">
        <label class="grid min-w-0 gap-1.5 text-xs font-medium text-muted-foreground">
          {{ $st('Category name') }}
          <Input
            v-model="categoryName"
            class="bg-background font-normal text-foreground"
            :disabled="busy"
            @update:model-value="categoryError = ''"
          />
        </label>
        <p v-if="categoryError" role="alert" class="break-words text-sm text-destructive">{{ categoryError }}</p>
        <div class="settings-toolbar justify-between">
          <Button v-if="category" size="sm" variant="ghost-destructive" :disabled="busy" @click="saveCategory(true)">
            <RiDeleteBinLine class="me-1.5 h-4 w-4 shrink-0" aria-hidden="true" />{{ $st('Delete') }}
          </Button>
          <Button type="submit" class="ms-auto" size="sm" :disabled="busy || !categoryDirty || !categoryName.trim()">
            {{ $st('Save') }}
          </Button>
        </div>
      </form>
    </FormDialog>
  </section>
</template>

<style scoped>
@reference '../../style.css';

.prompt-category-filter {
  @apply inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-md border border-border/70 px-2 py-1 text-xs font-medium transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50;
}

.prompt-category-filter[aria-pressed='true'] {
  @apply border-primary/30 bg-primary/10;
}

@container settings-workbench (min-width: 42rem) {
  .prompt-fragment-fields,
  .prompt-body-field {
    display: flex;
    flex-direction: column;
  }

  .prompt-fragment-fields {
    min-height: 100%;
  }

  .prompt-body-field,
  .prompt-body-field textarea {
    flex: 1 0 auto;
  }
}
</style>
