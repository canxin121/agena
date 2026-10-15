<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ActivitiesPanel = defineAsyncComponent(() => import('@/components/settings/ActivitiesPanel.vue'))
const AdvancedSettingsPanel = defineAsyncComponent(() => import('@/components/settings/AdvancedSettingsPanel.vue'))
const DiagnosticsPanel = defineAsyncComponent(() => import('@/components/settings/DiagnosticsPanel.vue'))
const MemoriesPanel = defineAsyncComponent(() => import('@/components/settings/MemoriesPanel.vue'))
const UsagePanel = defineAsyncComponent(() => import('@/components/settings/UsagePanel.vue'))

const panel = ref<{ refresh: () => Promise<unknown> } | null>(null)
defineExpose({
  refresh: async () => {
    await panel.value?.refresh()
  },
})
const pages = computed(() => buildSettingsSubpages('diagnostics'))
// The advanced editor writes the explicit layer selected in its workbench.
defineProps<{ activePage: string; scope: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="diagnostics"
    scope-layout="toolbar"
    :pages="pages"
    :active-page="activePage"
    :active-scope="scope"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.diagnostics"
    v-slot="{ activePage }"
  >
    <DiagnosticsPanel ref="panel" v-if="activePage === 'runtime'" />
    <AdvancedSettingsPanel ref="panel" v-else-if="activePage === 'advanced-settings'" :scope="scope" />
    <ActivitiesPanel ref="panel" v-else-if="activePage === 'activities'" />
    <MemoriesPanel ref="panel" v-else-if="activePage === 'memories'" />
    <UsagePanel ref="panel" v-else />
  </SettingsSectionWorkbench>
</template>
