<script setup lang="ts">
import { computed, ref } from 'vue'
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

function selectMemory(item: MemoryItem) {
  selectedName.value = item.name ?? item.file_name ?? item.path ?? null
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

      <div v-if="sortedItems.length > 0" class="space-y-2">
        <button
          v-for="item in sortedItems"
          :key="item.name ?? item.file_name ?? item.path"
          type="button"
          class="w-full rounded-md border border-border/60 bg-background/50 px-3 py-2.5 text-left transition-colors hover:bg-muted/30"
          :class="selectedName === (item.name ?? item.file_name ?? item.path) ? 'ring-1 ring-ring' : ''"
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
