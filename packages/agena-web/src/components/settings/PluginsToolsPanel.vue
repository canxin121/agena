<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const HarnessSettingsPanel = defineAsyncComponent(() => import('@/components/settings/HarnessSettingsPanel.vue'))
const McpServerControlPanel = defineAsyncComponent(() => import('@/components/settings/McpServerControlPanel.vue'))
const PluginMarketplacePanel = defineAsyncComponent(() => import('@/components/settings/PluginMarketplacePanel.vue'))
const PluginsPanel = defineAsyncComponent(() => import('@/components/settings/PluginsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('plugins-tools'))
</script>

<template>
  <SettingsSectionWorkbench
    section="plugins-tools"
    :title="$st('Plugins & Tools')"
    :description="
      $st('Operate the plugin runtime, expose Agena through MCP, and configure provider-native tool harnesses.')
    "
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['plugins-tools']"
    v-slot="{ activePage }"
  >
    <PluginsPanel v-if="activePage === 'plugin-workbench'" />
    <PluginMarketplacePanel v-else-if="activePage === 'marketplace'" />
    <McpServerControlPanel v-else-if="activePage === 'mcp-server'" />
    <HarnessSettingsPanel v-else />
  </SettingsSectionWorkbench>
</template>
