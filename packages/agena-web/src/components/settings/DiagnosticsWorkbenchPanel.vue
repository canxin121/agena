<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ActivitiesPanel = defineAsyncComponent(() => import('@/components/settings/ActivitiesPanel.vue'))
const AdvancedSettingsPanel = defineAsyncComponent(() => import('@/components/settings/AdvancedSettingsPanel.vue'))
const DiagnosticsPanel = defineAsyncComponent(() => import('@/components/settings/DiagnosticsPanel.vue'))
const MemoriesPanel = defineAsyncComponent(() => import('@/components/settings/MemoriesPanel.vue'))
const UsagePanel = defineAsyncComponent(() => import('@/components/settings/UsagePanel.vue'))

const pages = computed(() => buildSettingsSubpages('diagnostics'))
defineProps<{ activePage: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="diagnostics"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.diagnostics"
    v-slot="{ activePage }"
  >
    <DiagnosticsPanel v-if="activePage === 'runtime'" />
    <AdvancedSettingsPanel v-else-if="activePage === 'advanced-settings'" />
    <ActivitiesPanel v-else-if="activePage === 'activities'" />
    <MemoriesPanel v-else-if="activePage === 'memories'" />
    <UsagePanel v-else />
  </SettingsSectionWorkbench>
</template>
