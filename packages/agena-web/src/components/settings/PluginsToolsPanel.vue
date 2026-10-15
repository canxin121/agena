<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import { useRoute } from 'vue-router'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { pluginTabForSettingsView } from '@/components/settings/pluginSettingsNavigation'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const McpServerControlPanel = defineAsyncComponent(() => import('@/components/settings/McpServerControlPanel.vue'))
const PluginMarketplacePanel = defineAsyncComponent(() => import('@/components/settings/PluginMarketplacePanel.vue'))
const PluginsPanel = defineAsyncComponent(() => import('@/components/settings/PluginsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('plugins-tools'))
const route = useRoute()
const panel = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({
  refresh: async () => {
    await panel.value?.refresh()
  },
})
defineProps<{ activePage: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="plugins-tools"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['plugins-tools']"
    v-slot="{ activePage }"
  >
    <PluginsPanel
      v-if="activePage === 'plugin-workbench'"
      ref="panel"
      :tab="pluginTabForSettingsView(activePage, route.query.pluginTab)!"
    />
    <PluginMarketplacePanel ref="panel" v-else-if="activePage === 'marketplace'" />
    <McpServerControlPanel ref="panel" v-else />
  </SettingsSectionWorkbench>
</template>
