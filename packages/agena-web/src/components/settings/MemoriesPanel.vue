<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiRefreshLine } from '@remixicon/vue'

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
const selectedItems = computed(() =>
  selectableItems.value.filter((item) => selectedMemoryNames.value.has(memoryName(item))),
)
const allMemoriesSelected = computed(
  () =>
    selectableItems.value.length > 0 &&
    selectableItems.value.every((item) => selectedMemoryNames.value.has(memoryName(item))),
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
  const names = selectableItems.value.map(memoryName)
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

function selectMemory(item: MemoryItem) {
  selectedName.value = memoryName(item) || null
  editName.value = item.name ?? item.file_name ?? ''
  editDescription.value = item.description ?? ''
  editMemoryType.value = item.memory_type ?? ''
  editBody.value = item.body ?? ''
  editError.value = ''
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
</script>

<template>
  <div class="space-y-6">
    <div class="text-sm text-muted-foreground">
      {{ $st('Persistent memory entries the server keeps across sessions.') }}
    </div>
    <div v-if="workspaceRoot || directory" class="text-[11px] font-mono text-muted-foreground break-all">
      {{ [workspaceRoot, directory].filter(Boolean).join(' / ') }}
    </div>

    <div class="grid gap-3">
      <div v-if="loading && items.length === 0" class="text-sm text-muted-foreground">
        {{ $st('Loading memories...') }}
      </div>
      <div
        v-if="error"
        class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {{ error }}
      </div>
      <div v-if="!loading && sortedItems.length === 0" class="text-sm text-muted-foreground">
        {{ $st('No memories stored.') }}
      </div>

      <div
        v-if="selectableItems.length"
        class="flex flex-wrap items-center gap-2 rounded-md border border-border/60 bg-muted/10 px-3 py-2"
      >
        <label class="flex items-center gap-2 text-xs font-medium">
          <input type="checkbox" :checked="allMemoriesSelected" @change="toggleAllMemories" />
          {{ $st('Select all') }} ({{ selectableItems.length }})
        </label>
        <span class="text-xs text-muted-foreground">{{
          $st('{count} selected', { count: selectedItems.length })
        }}</span>
        <div class="ml-auto flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" :disabled="!selectedItems.length || editBusy" @click="clearMemorySelection">
            {{ $st('Select none') }}
          </Button>
          <ConfirmPopover
            :title="$st('Delete selected memories?')"
            :description="$st('Delete {count} selected memories?', { count: selectedItems.length })"
            :confirm-text="$st('Delete')"
            :cancel-text="$st('Cancel')"
            variant="destructive"
            @confirm="deleteSelectedMemories"
          >
            <Button variant="destructive" size="sm" :disabled="!selectedItems.length || editBusy">
              {{ $st('Delete selected memories ({count})', { count: selectedItems.length }) }}
            </Button>
          </ConfirmPopover>
        </div>
      </div>

      <div v-if="sortedItems.length > 0" class="space-y-2">
        <div
          v-for="item in sortedItems"
          :key="memoryName(item) || item.path"
          class="flex items-center gap-2 rounded-md border border-border/60 bg-background/50 px-3 py-2.5 transition-colors hover:bg-muted/30"
        >
          <label v-if="memoryName(item)" class="flex items-center justify-center">
            <input
              type="checkbox"
              :checked="selectedMemoryNames.has(memoryName(item))"
              :disabled="editBusy"
              @change="toggleMemorySelection(memoryName(item), $event)"
            />
            <span class="sr-only">{{ memoryName(item) }}</span>
          </label>
          <button
            type="button"
            class="min-w-0 flex-1 text-left"
            :class="selectedName === memoryName(item) ? 'ring-1 ring-ring' : ''"
            @click="selectMemory(item)"
          >
            <div class="text-sm font-semibold break-words">{{ item.name ?? item.file_name ?? item.path }}</div>
            <div v-if="item.description" class="mt-0.5 text-xs text-muted-foreground break-words">
              {{ item.description }}
            </div>
            <div
              v-if="item.memory_type"
              class="mt-1 inline-flex rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
            >
              {{ item.memory_type }}
            </div>
          </button>
        </div>
      </div>
    </div>

    <div v-if="selectedName" class="grid gap-3 rounded-lg border border-border bg-muted/10 p-4">
      <div class="text-sm font-medium">{{ $st('Edit memory') }}</div>
      <div class="grid gap-2">
        <label class="text-xs font-medium text-muted-foreground">{{ $st('Name') }}</label>
        <Input :model-value="editName" disabled class="h-10 font-mono" />
      </div>
      <div class="grid gap-2">
        <label class="text-xs font-medium text-muted-foreground">{{ $st('Description') }}</label>
        <Input v-model="editDescription" :disabled="editBusy" class="h-10" :placeholder="$st('Optional description')" />
      </div>
      <div class="grid gap-2">
        <label class="text-xs font-medium text-muted-foreground">{{ $st('Type') }}</label>
        <OptionPicker
          v-model="editMemoryType"
          :options="memoryTypeOptions"
          :title="$st('Memory type')"
          :empty-label="$st('Unclassified')"
          :include-empty="true"
          :disabled="editBusy"
        />
      </div>
      <div class="grid gap-2">
        <label class="text-xs font-medium text-muted-foreground">{{ $st('Body') }}</label>
        <textarea
          v-model="editBody"
          rows="6"
          :disabled="editBusy"
          class="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs"
        />
      </div>
      <div v-if="editError" class="text-xs text-destructive break-words">{{ editError }}</div>
      <div class="flex items-center justify-end gap-2">
        <ConfirmPopover
          :title="$st('Delete memory?')"
          :description="selectedName"
          :confirm-text="'Delete'"
          :cancel-text="'Cancel'"
          variant="destructive"
          @confirm="deleteSelected"
        >
          <Button
            variant="outline"
            size="sm"
            class="text-destructive border-destructive/30 hover:bg-destructive/10"
            :disabled="editBusy"
          >
            {{ $st('Delete') }}
          </Button>
        </ConfirmPopover>
        <Button variant="secondary" size="sm" :disabled="editBusy" @click="selectedName = null">{{
          $st('Cancel')
        }}</Button>
        <Button size="sm" :disabled="editBusy" @click="saveSelected">
          {{ editBusy ? 'Saving...' : $st('Save') }}
        </Button>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? $st('Refreshing...') : $st('Refresh')"
        :aria-label="loading ? $st('Refreshing...') : $st('Refresh')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
      <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
        {{ loading ? $st('Refreshing...') : $st('Refresh') }}
      </Button>
    </div>
  </div>
</template>
