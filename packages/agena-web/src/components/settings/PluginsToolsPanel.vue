<script setup lang="ts">
import HarnessSettingsPanel from '@/components/settings/HarnessSettingsPanel.vue'
import McpServerControlPanel from '@/components/settings/McpServerControlPanel.vue'
import PluginMarketplacePanel from '@/components/settings/PluginMarketplacePanel.vue'
import PluginsPanel from '@/components/settings/PluginsPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const pages = buildSettingsSubpages('plugins-tools')
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
