<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const McpServerControlPanel = defineAsyncComponent(() => import('@/components/settings/McpServerControlPanel.vue'))
const PluginMarketplacePanel = defineAsyncComponent(() => import('@/components/settings/PluginMarketplacePanel.vue'))
const PluginsPanel = defineAsyncComponent(() => import('@/components/settings/PluginsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('plugins-tools'))
</script>

<template>
  <SettingsSectionWorkbench
    section="plugins-tools"
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['plugins-tools']"
    v-slot="{ activePage }"
  >
    <PluginsPanel v-if="activePage === 'plugin-workbench'" />
    <PluginMarketplacePanel v-else-if="activePage === 'marketplace'" />
    <McpServerControlPanel v-else />
  </SettingsSectionWorkbench>
</template>
