<script setup lang="ts">
import { computed, ref } from 'vue'

import SettingsSidebarHeader from './SettingsSidebarHeader.vue'
import SettingsSidebarNavList from './SettingsSidebarNavList.vue'
import {
  buildSettingsSidebarGroups,
  normalizeSettingsSidebarQuery,
  type SettingsSidebarTab,
  type SettingsTab,
} from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    tabs?: SettingsSidebarTab[]
    activeTab: SettingsTab
    loading?: boolean
    isTouchPointer?: boolean
  }>(),
  {
    tabs: () => [],
    loading: false,
    isTouchPointer: false,
  },
)

const emit = defineEmits<{
  (e: 'refresh'): void
  (e: 'navigate-tab', id: SettingsTab): void
}>()

const query = ref('')
const queryNorm = computed(() => normalizeSettingsSidebarQuery(query.value))

const groups = computed(() =>
  buildSettingsSidebarGroups({
    query: queryNorm.value,
    tabs: props.tabs,
    activeTab: props.activeTab,
  }),
)

function setQuery(value: string) {
  query.value = String(value || '')
}

function submitQuery() {
  query.value = String(query.value || '')
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
        <SettingsSidebarNavList :groups="groups" @navigate-tab="(id) => emit('navigate-tab', id)" />
      </div>
    </div>
  </div>
</template>
