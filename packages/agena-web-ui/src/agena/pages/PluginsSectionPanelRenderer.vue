<script setup lang="ts">
import type { PluginsTab } from './runtimePageStateModel'
import type { usePluginsPageState } from './usePluginsPageState'

import PluginsInstalledPageContent from './PluginsInstalledPageContent.vue'
import PluginsMarketplacePageContent from './PluginsMarketplacePageContent.vue'

const props = defineProps<{
  activeTab: PluginsTab
  panels: ReturnType<typeof usePluginsPageState>['panels']
}>()
</script>

<template>
  <PluginsInstalledPageContent v-if="props.activeTab === 'installed'" :installed="props.panels.installed" />

  <PluginsMarketplacePageContent v-else :marketplace="props.panels.marketplace" />
</template>
