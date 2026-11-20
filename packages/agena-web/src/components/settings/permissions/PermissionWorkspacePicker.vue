<script setup lang="ts">
import { computed, onScopeDispose, ref } from 'vue'
import { RiArrowDownSLine, RiFolderLine } from '@remixicon/vue'

import GroupedPicker from '@/components/ui/GroupedPicker.vue'
import type { OptionMenuGroup } from '@/components/ui/optionMenu.types'
import { fetchAgenaWorkspaces } from '@/stores/directorySessionStore'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ modelValue: string; configPath?: string; disabled?: boolean }>()
const emit = defineEmits<{ select: [id: string] }>()

const serverId = 'server'
const query = ref('')
const page = ref(1)
const hasMore = ref(false)
const loading = ref(false)
const error = ref('')
const workspaces = ref<{ id: string; path: string }[]>([])
let controller: AbortController | undefined
let searchTimer: ReturnType<typeof setTimeout> | undefined

const selectedId = computed(() => props.modelValue || serverId)
const serverLabel = computed(() => st('Server workspace'))
function workspaceName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || path
}

const label = computed(() => {
  const path = props.configPath
  if (!path) return props.modelValue ? st('Workspace') : serverLabel.value
  const root = path.replace(/[\\/]\.agena[\\/]agena\.json$/, '')
  return `${workspaceName(root || '/')} · ${path}`
})
const groups = computed<OptionMenuGroup[]>(() => {
  const items = workspaces.value.map(({ id, path }) => ({ id, label: workspaceName(path), description: path }))
  if (!query.value.trim() || serverLabel.value.toLowerCase().includes(query.value.trim().toLowerCase())) {
    return [{ items: [{ id: serverId, label: serverLabel.value }, ...items] }]
  }
  return items.length ? [{ items }] : []
})

async function loadPage(next = page.value) {
  clearTimeout(searchTimer)
  controller?.abort()
  controller = new AbortController()
  const { signal } = controller
  const search = query.value.trim()
  loading.value = true
  error.value = ''
  try {
    const result = await fetchAgenaWorkspaces({ limit: 30, page: next - 1, search, signal })
    if (signal.aborted) return
    workspaces.value = result.entries.map(({ id, path }) => ({ id: String(id), path }))
    page.value = next
    hasMore.value = result.hasMore
  } catch (reason) {
    if (!signal.aborted) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (!signal.aborted) loading.value = false
  }
}

function search(value: string) {
  clearTimeout(searchTimer)
  controller?.abort()
  query.value = value
  page.value = 1
  hasMore.value = false
  workspaces.value = []
  loading.value = true
  searchTimer = setTimeout(() => void loadPage(1), 180)
}

function select(id: string) {
  if (id === selectedId.value) return
  if (id === serverId || workspaces.value.some((workspace) => workspace.id === id)) {
    emit('select', id === serverId ? '' : id)
  }
}

onScopeDispose(() => {
  clearTimeout(searchTimer)
  controller?.abort()
})
</script>

<template>
  <div class="min-w-0 flex-1" data-permission-workspace-picker>
    <GroupedPicker
      block
      size="sm"
      trigger-class="text-[13px]"
      :icon="RiFolderLine"
      :label="label"
      :selected-id="selectedId"
      :title="$st('Workspace')"
      :groups="groups"
      :disabled="disabled"
      :query="query"
      filter-mode="external"
      :search-placeholder="$st('Search projects...')"
      :empty-text="loading ? $st('Loading…') : $st('No projects found.')"
      :helper-text="error"
      :page="page"
      :page-count="page + Number(hasMore)"
      :page-loading="loading"
      :loading="loading && !groups.length"
      refreshable
      :refreshing="loading"
      :refresh-label="$st('Retry')"
      @update:open="$event && loadPage()"
      @update:query="search"
      @request-page="loadPage"
      @refresh="loadPage()"
      @select="select"
    >
      <template #trailing
        ><RiArrowDownSLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true"
      /></template>
    </GroupedPicker>
  </div>
</template>
