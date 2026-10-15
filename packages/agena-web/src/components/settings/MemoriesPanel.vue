<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiRefreshLine, RiBookOpenLine, RiEditLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { confirmAction } from '@/lib/appConfirm'
import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '../../lib/api'
import { useToastsStore } from '../../stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'

const MEMORY_DELETE_BATCH_SIZE = 1_000

type MemoryItem = {
  name?: string
  file_name?: string
  path?: string
  description?: string
  memory_type?: string
  body?: string
}

type MemoriesOverview = {
  workspace_root?: string
  directory?: string
  items?: MemoryItem[]
}

const toasts = useToastsStore()

const items = ref<MemoryItem[]>([])
const workspaceRoot = ref('')
const directory = ref('')

const selectedName = ref<string | null>(null)
const selectedMemoryNames = ref<Set<string>>(new Set())
const editName = ref('')
const editDescription = ref('')
const editMemoryType = ref('')
const editBody = ref('')
const editBusy = ref(false)
const editError = ref('')
const searchQuery = ref('')
const editorElement = ref<HTMLElement | null>(null)
const originalMemory = ref<MemoryItem | null>(null)
const dirty = computed(() =>
  Boolean(
    selectedName.value &&
    originalMemory.value &&
    (editDescription.value !== (originalMemory.value.description ?? '') ||
      editMemoryType.value !== (originalMemory.value.memory_type ?? '') ||
      editBody.value !== (originalMemory.value.body ?? '')),
  ),
)

const memoryTypeOptions = computed(() => [
  { value: 'user', label: st('User'), description: st('Stable user preferences and personal context.') },
  { value: 'feedback', label: st('Feedback'), description: st('Corrections and feedback from prior work.') },
  { value: 'project', label: st('Project'), description: st('Project-specific facts and conventions.') },
  { value: 'reference', label: st('Reference'), description: st('Reusable reference information.') },
  { value: 'other', label: st('Other'), description: st('Memory that does not fit another category.') },
])

const sortedItems = computed(() =>
  [...items.value].sort((a, b) => String(a.name || '').localeCompare(String(b.name || ''))),
)
const selectableItems = computed(() => sortedItems.value.filter((item) => memoryName(item)))
const filteredItems = computed(() => {
  const query = searchQuery.value.trim().toLowerCase()
  return sortedItems.value.filter(
    (item) => !query || [memoryName(item), item.description, item.memory_type].join(' ').toLowerCase().includes(query),
  )
})
const visibleSelectableItems = computed(() => filteredItems.value.filter((item) => memoryName(item)))
const selectedItems = computed(() =>
  selectableItems.value.filter((item) => selectedMemoryNames.value.has(memoryName(item))),
)
const allMemoriesSelected = computed(
  () =>
    visibleSelectableItems.value.length > 0 &&
    visibleSelectableItems.value.every((item) => selectedMemoryNames.value.has(memoryName(item))),
)

function memoryName(item: MemoryItem): string {
  return String(item.name || item.file_name || '')
    .trim()
    .replace(/\.md$/i, '')
}

watch(selectableItems, (available) => {
  const allowed = new Set(available.map(memoryName))
  selectedMemoryNames.value = new Set([...selectedMemoryNames.value].filter((name) => allowed.has(name)))
})

function toggleMemorySelection(name: string, event: Event) {
  const next = new Set(selectedMemoryNames.value)
  if ((event.target as HTMLInputElement).checked) next.add(name)
  else next.delete(name)
  selectedMemoryNames.value = next
}

function toggleAllMemories() {
  const names = visibleSelectableItems.value.map(memoryName)
  if (allMemoriesSelected.value) {
    const removing = new Set(names)
    selectedMemoryNames.value = new Set([...selectedMemoryNames.value].filter((name) => !removing.has(name)))
  } else {
    selectedMemoryNames.value = new Set([...selectedMemoryNames.value, ...names])
  }
}

function clearMemorySelection() {
  selectedMemoryNames.value = new Set()
}

async function selectMemory(item: MemoryItem) {
  if (selectedName.value === memoryName(item)) return
  if (!(await canLeave())) return
  originalMemory.value = { ...item }
  selectedName.value = memoryName(item) || null
  editName.value = item.name ?? item.file_name ?? ''
  editDescription.value = item.description ?? ''
  editMemoryType.value = item.memory_type ?? ''
  editBody.value = item.body ?? ''
  editError.value = ''
  await nextTick()
  if (window.matchMedia('(max-width: 1279px)').matches)
    editorElement.value?.scrollIntoView({ block: 'start', behavior: 'smooth' })
}

const { loading, error, refresh } = usePaneRead(
  async (signal) => {
    const overview = await apiJson<MemoriesOverview>('/api/v1/memories/overview', { signal })
    let items = Array.isArray(overview?.items) ? overview.items : []
    if (!items.length) {
      const flat = await apiJson<MemoryItem[]>('/api/v1/memories', { signal }).catch(() => null)
      if (Array.isArray(flat)) items = flat
    }
    return { overview, items }
  },
  ({ overview, items: next }) => {
    workspaceRoot.value = overview?.workspace_root ?? ''
    directory.value = overview?.directory ?? ''
    items.value = next
  },
)

async function saveSelected() {
  const name = selectedName.value
  if (!name || editBusy.value) return
  editBusy.value = true
  editError.value = ''
  try {
    await apiJson(`/api/v1/memories/${encodeURIComponent(name)}`, {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        description: editDescription.value.trim(),
        memory_type: editMemoryType.value.trim() || null,
        body: editBody.value,
      }),
    })
    toasts.push('success', st('Memory updated'))
    selectedName.value = null
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    editError.value = msg
    toasts.push('error', msg)
  } finally {
    editBusy.value = false
  }
}

async function deleteSelected() {
  const name = selectedName.value
  if (!name || editBusy.value) return
  editBusy.value = true
  editError.value = ''
  try {
    await apiJson(`/api/v1/memories/${encodeURIComponent(name)}`, { method: 'DELETE' })
    toasts.push('success', st('Memory deleted'))
    selectedName.value = null
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    editError.value = msg
    toasts.push('error', msg)
    await refresh().catch(() => {})
    if (selectedName.value && !items.value.some((item) => memoryName(item) === selectedName.value)) {
      selectedName.value = null
    }
  } finally {
    editBusy.value = false
  }
}

async function deleteSelectedMemories() {
  const names = [...new Set(selectedItems.value.map(memoryName))]
  if (!names.length || editBusy.value) return
  editBusy.value = true
  editError.value = ''
  try {
    let deletedCount = 0
    for (let offset = 0; offset < names.length; offset += MEMORY_DELETE_BATCH_SIZE) {
      const batch = names.slice(offset, offset + MEMORY_DELETE_BATCH_SIZE)
      const deleted = await apiJson<string[]>('/api/v1/memories/batch-delete', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ names: batch }),
      })
      deletedCount += Array.isArray(deleted) ? deleted.length : batch.length
    }
    if (selectedName.value && names.includes(selectedName.value)) selectedName.value = null
    clearMemorySelection()
    toasts.push('success', st('{count} memories deleted', { count: deletedCount }))
    await refresh()
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    editError.value = msg
    toasts.push('error', msg)
    await refresh().catch(() => {})
    if (selectedName.value && !items.value.some((item) => memoryName(item) === selectedName.value)) {
      selectedName.value = null
    }
  } finally {
    editBusy.value = false
  }
}
function discardDraft() {
  if (!originalMemory.value) return
  editDescription.value = originalMemory.value.description ?? ''
  editMemoryType.value = originalMemory.value.memory_type ?? ''
  editBody.value = originalMemory.value.body ?? ''
  editError.value = ''
}
async function canLeave() {
  if (editBusy.value) return false
  if (!dirty.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
async function closeEditor() {
  if (await canLeave()) selectedName.value = null
}
onBeforeRouteUpdate((to, from) => {
  if (to.path === from.path && to.query.view === from.query.view) return true
  return canLeave()
})
onBeforeRouteLeave(canLeave)
defineExpose({ refresh })
</script>

<template>
  <div
    class="grid min-w-0 items-start gap-4"
    :class="selectedName ? 'xl:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)]' : ''"
  >
    <SettingsCard :title="$st('Memory library')" :icon="RiBookOpenLine" :count="items.length">
      <template #actions
        ><IconButton
          variant="outline"
          size="sm"
          :tooltip="$st('Refresh')"
          :aria-label="$st('Refresh')"
          :disabled="loading || editBusy"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" /></IconButton
      ></template>
      <SearchInput
        v-model="searchQuery"
        class="mb-3"
        :placeholder="$st('Search memories')"
        :input-aria-label="$st('Search memories')"
        :show-search-button="false"
      />
      <p
        v-if="error"
        role="alert"
        class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
      >
        {{ error }}
      </p>
      <p v-if="editError && !selectedName" role="alert" class="mb-3 break-words text-xs text-destructive">
        {{ editError }}
      </p>
      <div v-if="items.length" class="mb-3 flex flex-wrap items-center gap-2 rounded-lg bg-muted/20 p-2.5">
        <label class="flex items-center gap-2 text-xs font-medium"
          ><input
            type="checkbox"
            :checked="allMemoriesSelected"
            :disabled="editBusy || !visibleSelectableItems.length"
            @change="toggleAllMemories"
          />{{ $st('Select visible') }} ({{ visibleSelectableItems.length }})</label
        >
        <div v-if="selectedItems.length" class="ml-auto flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" :disabled="editBusy" @click="clearMemorySelection">{{
            $st('Select none')
          }}</Button>
          <ConfirmPopover
            :title="$st('Delete selected memories?')"
            :description="$st('Delete {count} selected memories?', { count: selectedItems.length })"
            :confirm-text="$st('Delete')"
            :cancel-text="$st('Cancel')"
            variant="destructive"
            @confirm="deleteSelectedMemories"
          >
            <Button
              variant="outline"
              size="sm"
              class="h-auto whitespace-normal text-destructive"
              :disabled="editBusy"
              >{{ $st('Delete selected memories ({count})', { count: selectedItems.length }) }}</Button
            >
          </ConfirmPopover>
        </div>
      </div>
      <p v-if="loading && !items.length" class="py-6 text-center text-sm text-muted-foreground">
        {{ $st('Loading memories...') }}
      </p>
      <div
        v-else-if="!filteredItems.length"
        class="rounded-lg border border-dashed border-border/60 bg-muted/10 px-4 py-8 text-center"
      >
        <RiBookOpenLine class="mx-auto mb-2 h-6 w-6 text-muted-foreground/60" />
        <p class="text-sm text-muted-foreground">
          {{ items.length ? $st('No matching memories.') : $st('No memories stored.') }}
        </p>
        <Button v-if="searchQuery" variant="ghost" size="sm" class="mt-2" @click="searchQuery = ''">{{
          $st('Clear filters')
        }}</Button>
      </div>
      <div v-else class="space-y-2" :class="selectedName ? 'xl:max-h-[36rem] xl:overflow-y-auto' : ''">
        <div
          v-for="item in filteredItems"
          :key="memoryName(item) || item.path"
          class="flex min-w-0 items-start gap-3 rounded-lg border p-3 transition-colors"
          :class="
            selectedName === memoryName(item)
              ? 'border-primary/30 bg-primary/5'
              : 'border-border/50 bg-background/30 hover:bg-muted/20'
          "
        >
          <input
            v-if="memoryName(item)"
            type="checkbox"
            class="mt-1 shrink-0"
            :aria-label="memoryName(item)"
            :checked="selectedMemoryNames.has(memoryName(item))"
            :disabled="editBusy"
            @change="toggleMemorySelection(memoryName(item), $event)"
          />
          <button
            type="button"
            class="min-w-0 flex-1 rounded text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :disabled="editBusy"
            :aria-pressed="selectedName === memoryName(item)"
            @click="selectMemory(item)"
          >
            <span class="flex flex-wrap items-center gap-2"
              ><span class="min-w-0 max-w-full break-words text-sm font-semibold">{{
                item.name ?? item.file_name ?? item.path
              }}</span
              ><span
                v-if="item.memory_type"
                class="max-w-full break-words rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                >{{
                  memoryTypeOptions.find((option) => option.value === item.memory_type)?.label || item.memory_type
                }}</span
              ></span
            >
            <span
              v-if="item.description"
              class="mt-1 block break-words text-xs leading-relaxed text-muted-foreground"
              >{{ item.description }}</span
            >
          </button>
          <RiEditLine class="mt-1 h-3.5 w-3.5 shrink-0 text-muted-foreground/50" aria-hidden="true" />
        </div>
      </div>
      <details
        v-if="workspaceRoot || directory"
        class="mt-4 border-t border-border/50 pt-3 text-xs text-muted-foreground"
      >
        <summary class="cursor-pointer">{{ $st('Storage location') }}</summary>
        <dl class="mt-3 space-y-2">
          <div v-if="workspaceRoot">
            <dt>{{ $st('Workspace:') }}</dt>
            <dd class="mt-1 break-all font-mono text-[11px]">{{ workspaceRoot }}</dd>
          </div>
          <div v-if="directory">
            <dt>{{ $st('Directory') }}</dt>
            <dd class="mt-1 break-all font-mono text-[11px]">{{ directory }}</dd>
          </div>
        </dl>
      </details>
    </SettingsCard>
    <div v-if="selectedName" ref="editorElement" class="min-w-0 scroll-mt-4">
      <SettingsCard :title="$st('Edit memory')" :icon="RiEditLine">
        <template #actions
          ><Button variant="ghost" size="sm" :disabled="editBusy" @click="closeEditor">{{
            $st('Close')
          }}</Button></template
        >
        <div class="space-y-4">
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="min-w-0 space-y-2"
              ><span class="block text-xs font-medium text-muted-foreground">{{ $st('Name') }}</span
              ><Input :model-value="editName" disabled class="font-mono text-sm"
            /></label>
            <div class="min-w-0 space-y-2">
              <div class="text-xs font-medium text-muted-foreground">{{ $st('Type') }}</div>
              <OptionPicker
                v-model="editMemoryType"
                :options="memoryTypeOptions"
                :title="$st('Memory type')"
                :empty-label="$st('Unclassified')"
                :include-empty="true"
                :disabled="editBusy"
              />
            </div>
          </div>
          <label class="block space-y-2"
            ><span class="block text-xs font-medium text-muted-foreground">{{ $st('Description') }}</span
            ><Input v-model="editDescription" :disabled="editBusy" :placeholder="$st('Optional description')"
          /></label>
          <label class="block space-y-2"
            ><span class="block text-xs font-medium text-muted-foreground">{{ $st('Body') }}</span
            ><textarea
              v-model="editBody"
              rows="12"
              :disabled="editBusy"
              class="w-full min-w-0 resize-y rounded-lg border border-input bg-background/60 p-3 font-mono text-xs leading-6 outline-none focus:border-ring"
            />
          </label>
          <p v-if="editError" role="alert" class="break-words text-xs text-destructive">{{ editError }}</p>
          <ConfirmPopover
            :title="$st('Delete memory?')"
            :description="selectedName"
            :confirm-text="$st('Delete')"
            :cancel-text="$st('Cancel')"
            variant="destructive"
            @confirm="deleteSelected"
          >
            <Button variant="ghost" size="sm" class="text-destructive hover:bg-destructive/10" :disabled="editBusy">{{
              $st('Delete')
            }}</Button>
          </ConfirmPopover>
          <SettingsSaveBar
            v-if="dirty || editBusy"
            :dirty="dirty"
            :saving="editBusy"
            @save="saveSelected"
            @discard="discardDraft"
          />
        </div>
      </SettingsCard>
    </div>
  </div>
</template>
