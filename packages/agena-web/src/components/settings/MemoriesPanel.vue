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
import { apiJson, ApiError } from '../../lib/api'
import MemoryMetadataView from '@/components/MemoryMetadataView.vue'
import type { MemoryKind, MemoryMetadata } from '@/generated/agenaState'
import { isMemoryKind, memoryBatchDeleteRequest, memoryRevision, memoryWriteRequest } from '@/lib/memoryRequests'
import { useToastsStore } from '../../stores/toasts'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneRead } from '@/composables/usePaneRead'

const MEMORY_DELETE_BATCH_SIZE = 1_000

type MemoryItem = {
  name: string
  file_name: string
  path: string
  description: string
  body: string
  sha256: string
  metadata: MemoryMetadata
}

type MemoriesOverview = {
  workspace_root: string
  directory: string
  items: MemoryItem[]
}

const toasts = useToastsStore()

const items = ref<MemoryItem[]>([])
const workspaceRoot = ref('')
const directory = ref('')

const selectedName = ref<string | null>(null)
const selectedMemoryNames = ref<Set<string>>(new Set())
const editName = ref('')
const editDescription = ref('')
const editKind = ref<MemoryKind>('fact')
const editBody = ref('')
const editBusy = ref(false)
const editError = ref('')
const editConflict = ref(false)
const searchQuery = ref('')
const editorElement = ref<HTMLElement | null>(null)
const originalMemory = ref<MemoryItem | null>(null)
const dirty = computed(() =>
  Boolean(
    selectedName.value &&
    originalMemory.value &&
    (editDescription.value !== originalMemory.value.description ||
      editKind.value !== originalMemory.value.metadata.kind ||
      editBody.value !== originalMemory.value.body),
  ),
)

const memoryKindOptions = computed(() => [
  { value: 'fact', label: st('Fact'), description: st('Facts and decisions relevant to future work.') },
  {
    value: 'preference',
    label: st('Preference'),
    description: st('Stable preferences for how the agent should work.'),
  },
  { value: 'episode', label: st('Episode'), description: st('Outcomes and lessons from completed work.') },
  { value: 'procedure', label: st('Procedure'), description: st('Reusable steps for recurring tasks.') },
  { value: 'task', label: st('Task'), description: st('Current goals, progress, and next actions.') },
])

function setMemoryKind(kind: string) {
  if (isMemoryKind(kind)) editKind.value = kind
}

const sortedItems = computed(() =>
  [...items.value].sort((a, b) => {
    const rank = (item: MemoryItem) => (item.metadata.status === 'active' ? 0 : 1)
    return rank(a) - rank(b) || b.metadata.updated_at_ms - a.metadata.updated_at_ms || a.name.localeCompare(b.name)
  }),
)
const selectableItems = sortedItems
const filteredItems = computed(() => {
  const query = searchQuery.value.trim().toLowerCase()
  return sortedItems.value.filter(
    (item) =>
      !query || [memoryName(item), item.description, item.metadata.kind].join(' ').toLowerCase().includes(query),
  )
})
const visibleSelectableItems = filteredItems
const selectedItems = computed(() =>
  selectableItems.value.filter((item) => selectedMemoryNames.value.has(memoryName(item))),
)
const allMemoriesSelected = computed(
  () =>
    visibleSelectableItems.value.length > 0 &&
    visibleSelectableItems.value.every((item) => selectedMemoryNames.value.has(memoryName(item))),
)

function memoryName(item: MemoryItem): string {
  return item.file_name.replace(/\.md$/, '')
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
  editConflict.value = false
  originalMemory.value = { ...item }
  selectedName.value = memoryName(item)
  editName.value = item.name
  editDescription.value = item.description
  editKind.value = item.metadata.kind
  editBody.value = item.body
  editError.value = ''
  await nextTick()
  if (window.matchMedia('(max-width: 1279px)').matches)
    editorElement.value?.scrollIntoView({ block: 'start', behavior: 'smooth' })
}

const { loading, error, refresh } = usePaneRead(
  (signal) => apiJson<MemoriesOverview>('/api/v1/memories/overview', { signal }),
  (overview) => {
    workspaceRoot.value = overview.workspace_root
    directory.value = overview.directory
    items.value = overview.items
  },
)

async function saveSelected() {
  const name = selectedName.value
  const original = originalMemory.value
  if (!name || !original || editBusy.value || editConflict.value) return
  editBusy.value = true
  editError.value = ''
  try {
    await apiJson(`/api/v1/memories/${encodeURIComponent(name)}`, {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        memoryWriteRequest(editKind.value, editBody.value, editDescription.value.trim(), original.sha256),
      ),
    })
    toasts.push('success', st('Memory updated'))
    selectedName.value = null
    await refresh()
  } catch (err) {
    editConflict.value = err instanceof ApiError && err.status === 409
    const msg = editConflict.value
      ? st('This memory changed. Reload the latest version before editing.')
      : err instanceof Error
        ? err.message
        : String(err)
    editError.value = msg
    toasts.push('error', msg)
  } finally {
    editBusy.value = false
  }
}

async function deleteSelected() {
  const name = selectedName.value
  const original = originalMemory.value
  if (!name || !original || editBusy.value || editConflict.value) return
  editBusy.value = true
  editError.value = ''
  try {
    const query = `?expected_sha256=${encodeURIComponent(memoryRevision(original.sha256))}`
    await apiJson(`/api/v1/memories/${encodeURIComponent(name)}${query}`, { method: 'DELETE' })
    toasts.push('success', st('Memory deleted'))
    selectedName.value = null
    await refresh()
  } catch (err) {
    editConflict.value = err instanceof ApiError && err.status === 409
    const msg = editConflict.value
      ? st('This memory changed. Reload the latest version before editing.')
      : err instanceof Error
        ? err.message
        : String(err)
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
  const records = selectedItems.value.map((item) => ({ name: memoryName(item), sha256: item.sha256 }))
  const names = records.map((record) => record.name)
  if (!records.length || editBusy.value) return
  editBusy.value = true
  editError.value = ''
  try {
    let deletedCount = 0
    for (let offset = 0; offset < records.length; offset += MEMORY_DELETE_BATCH_SIZE) {
      const batch = records.slice(offset, offset + MEMORY_DELETE_BATCH_SIZE)
      const deleted = await apiJson<string[]>('/api/v1/memories/batch-delete', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(memoryBatchDeleteRequest(batch)),
      })
      deletedCount += deleted.length
    }
    if (selectedName.value && names.includes(selectedName.value)) selectedName.value = null
    clearMemorySelection()
    toasts.push('success', st('{count} memories deleted', { count: deletedCount }))
    await refresh()
  } catch (err) {
    editConflict.value = err instanceof ApiError && err.status === 409
    const msg = editConflict.value
      ? st('This memory changed. Reload the latest version before editing.')
      : err instanceof Error
        ? err.message
        : String(err)
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
async function reloadSelected() {
  if (editBusy.value || !selectedName.value) return
  const name = selectedName.value
  editBusy.value = true
  try {
    const latest = await apiJson<MemoryItem>(`/api/v1/memories/${encodeURIComponent(name)}`)
    items.value = items.value.map((item) => (memoryName(item) === name ? latest : item))
    originalMemory.value = { ...latest }
    editDescription.value = latest.description
    editKind.value = latest.metadata.kind
    editBody.value = latest.body
    editError.value = ''
    editConflict.value = false
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    editError.value = msg
    toasts.push('error', msg)
    if (err instanceof ApiError && err.status === 404) {
      selectedName.value = null
      originalMemory.value = null
      items.value = items.value.filter((item) => memoryName(item) !== name)
      editConflict.value = false
    }
  } finally {
    editBusy.value = false
  }
}

function discardDraft() {
  if (!originalMemory.value) return
  editDescription.value = originalMemory.value.description
  editKind.value = originalMemory.value.metadata.kind
  editBody.value = originalMemory.value.body
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
          :key="memoryName(item)"
          class="flex min-w-0 items-start gap-3 rounded-lg border p-3 transition-colors"
          :class="
            selectedName === memoryName(item)
              ? 'border-primary/30 bg-primary/5'
              : 'border-border/50 bg-background/30 hover:bg-muted/20'
          "
        >
          <input
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
              ><span class="min-w-0 max-w-full break-words text-sm font-semibold">{{ item.name }}</span></span
            >
            <span
              v-if="item.description"
              class="mt-1 block break-words text-xs leading-relaxed text-muted-foreground"
              >{{ item.description }}</span
            >
            <MemoryMetadataView :metadata="item.metadata" compact />
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
    <div v-if="selectedName && originalMemory" ref="editorElement" class="min-w-0 scroll-mt-4">
      <SettingsCard :title="$st('Edit memory')" :icon="RiEditLine">
        <template #actions
          ><Button variant="ghost" size="sm" :disabled="editBusy" @click="closeEditor">{{
            $st('Close')
          }}</Button></template
        >
        <div class="space-y-4">
          <MemoryMetadataView :metadata="originalMemory.metadata" />
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="min-w-0 space-y-2"
              ><span class="block text-xs font-medium text-muted-foreground">{{ $st('Name') }}</span
              ><Input :model-value="editName" disabled class="font-mono text-sm"
            /></label>
            <div class="min-w-0 space-y-2">
              <div class="text-xs font-medium text-muted-foreground">{{ $st('Memory category') }}</div>
              <OptionPicker
                :model-value="editKind"
                :options="memoryKindOptions"
                :title="$st('Memory category')"
                :include-empty="false"
                :disabled="editBusy"
                @update:model-value="setMemoryKind"
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
          <Button
            v-if="editConflict"
            variant="outline"
            size="sm"
            :disabled="editBusy || loading"
            @click="reloadSelected"
            >{{ $st('Reload latest version') }}</Button
          >
          <ConfirmPopover
            :title="$st('Delete memory?')"
            :description="selectedName"
            :confirm-text="$st('Delete')"
            :cancel-text="$st('Cancel')"
            variant="destructive"
            @confirm="deleteSelected"
          >
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive hover:bg-destructive/10"
              :disabled="editBusy || editConflict"
              >{{ $st('Delete') }}</Button
            >
          </ConfirmPopover>
          <SettingsSaveBar
            v-if="(dirty || editBusy) && !editConflict"
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
