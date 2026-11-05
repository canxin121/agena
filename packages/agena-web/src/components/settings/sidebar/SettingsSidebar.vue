<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import SettingsSidebarHeader from './SettingsSidebarHeader.vue'
import SettingsSidebarNavList from './SettingsSidebarNavList.vue'
import {
  buildSettingsSidebarGroups,
  normalizeSettingsSidebarQuery,
  type SettingsSidebarDestination,
  type SettingsSidebarTab,
  type SettingsSidebarTabRow,
  type SettingsTab,
} from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    tabs?: SettingsSidebarTab[]
    activeTab: SettingsTab
    activeView?: string
    activeScope?: string
    loading?: boolean
    isTouchPointer?: boolean
  }>(),
  {
    tabs: () => [],
    activeView: '',
    activeScope: '',
    loading: false,
    isTouchPointer: false,
  },
)

const emit = defineEmits<{
  (e: 'refresh'): void
  (e: 'navigate', destination: SettingsSidebarDestination): void
}>()

const query = ref('')
const expandedNodeKeys = ref<Set<string>>(new Set())
const queryNorm = computed(() => normalizeSettingsSidebarQuery(query.value))

// One visual category may contain pages from several legacy route sections.
// Open the category that owns the current (section, view) destination.
function activeBranchKeys(): string[] {
  const activeView = normalizeSettingsSidebarQuery(props.activeView || '')
  const category = props.tabs.find((tab) =>
    (tab.children || []).some(
      (node) =>
        node.section === props.activeTab &&
        (normalizeSettingsSidebarQuery(node.view || '') === activeView || node.relatedViews?.includes(activeView)),
    ),
  )
  if (!category) return []
  const keys = [category.id]
  const task = category.children?.find(
    (node) =>
      node.section === props.activeTab &&
      (normalizeSettingsSidebarQuery(node.view || '') === activeView || node.relatedViews?.includes(activeView)),
  )
  if (task?.children?.length) keys.push(`${category.id}/${task.id}`)
  return keys
}

watch(
  () => [props.activeTab, props.activeView, props.tabs] as const,
  () => {
    const next = new Set(expandedNodeKeys.value)
    for (const key of activeBranchKeys()) next.add(key)
    expandedNodeKeys.value = next
  },
  { immediate: true },
)

const groups = computed(() =>
  buildSettingsSidebarGroups({
    query: queryNorm.value,
    tabs: props.tabs,
    activeTab: props.activeTab,
    activeView: props.activeView,
    activeScope: props.activeScope,
    expandedNodeKeys: expandedNodeKeys.value,
  }),
)

function setQuery(value: string) {
  query.value = String(value || '')
}

function submitQuery() {
  query.value = String(query.value || '')
}

function toggleNode(key: string) {
  const next = new Set(expandedNodeKeys.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expandedNodeKeys.value = next
}

function activateRow(row: SettingsSidebarTabRow) {
  if (row.hasChildren) {
    toggleNode(row.key)
    return
  }
  if (!row.view) return
  // A scope row carries its page plus the layer it opens.
  emit(
    'navigate',
    row.scope ? { section: row.section, view: row.view, scope: row.scope } : { section: row.section, view: row.view },
  )
}
</script>

<template>
  <div class="flex h-full flex-col overflow-hidden bg-sidebar">
    <SettingsSidebarHeader
      :query="query"
      :loading="loading"
      :is-touch-pointer="isTouchPointer"
      @update:query="setQuery"
      @submit-query="submitQuery"
      @refresh="emit('refresh')"
    />

    <div class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-2">
      <div class="flex min-h-full flex-col">
        <SettingsSidebarNavList :groups="groups" @activate-row="activateRow" />
      </div>
    </div>
  </div>
</template>
