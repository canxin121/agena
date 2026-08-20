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
    loading?: boolean
    isTouchPointer?: boolean
  }>(),
  {
    tabs: () => [],
    activeView: '',
    loading: false,
    isTouchPointer: false,
  },
)

const emit = defineEmits<{
  (e: 'refresh'): void
  (e: 'navigate', destination: SettingsSidebarDestination): void
}>()

const query = ref('')
const expandedNodeKeys = ref<Set<string>>(new Set([props.activeTab]))
const queryNorm = computed(() => normalizeSettingsSidebarQuery(query.value))

watch(
  () => props.activeTab,
  (activeTab) => {
    const next = new Set(expandedNodeKeys.value)
    next.add(activeTab)
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
  emit('navigate', { section: row.section, view: row.view })
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
