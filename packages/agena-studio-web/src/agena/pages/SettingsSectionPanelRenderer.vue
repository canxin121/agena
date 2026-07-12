<script setup lang="ts">
import type { SettingsTab } from './runtimePageStateModel'
import type { useSettingsPageState } from './useSettingsPageState'

import SettingsAgentsPageContent from './SettingsAgentsPageContent.vue'
import SettingsConfigurationPageContent from './SettingsConfigurationPageContent.vue'
import SettingsDesktopPageContent from './SettingsDesktopPageContent.vue'
import SettingsMemoryPageContent from './SettingsMemoryPageContent.vue'
import SettingsPermissionsPageContent from './SettingsPermissionsPageContent.vue'
import SettingsPluginsPageContent from './SettingsPluginsPageContent.vue'
import SettingsProvidersPageContent from './SettingsProvidersPageContent.vue'

const props = defineProps<{
  activeTab: SettingsTab
  loading: boolean
  load: () => Promise<void>
  panels: ReturnType<typeof useSettingsPageState>['panels']
}>()
</script>

<template>
  <SettingsProvidersPageContent v-if="props.activeTab === 'providers'" :providers="props.panels.providers" />

  <SettingsAgentsPageContent v-else-if="props.activeTab === 'agents'" :agents="props.panels.agents" />

  <SettingsPluginsPageContent v-else-if="props.activeTab === 'plugins'" :plugins="props.panels.plugins" />

  <SettingsConfigurationPageContent
    v-else-if="props.activeTab === 'configuration'"
    :configuration="props.panels.configuration"
  />

  <SettingsMemoryPageContent v-else-if="props.activeTab === 'memory'" :memory="props.panels.memory" />

  <SettingsPermissionsPageContent
    v-else-if="props.activeTab === 'permissions'"
    :loading="props.loading"
    :load="props.load"
    :permissions="props.panels.permissions"
  />

  <SettingsDesktopPageContent
    v-else-if="props.activeTab === 'desktop'"
    :loading="props.loading"
    :desktop="props.panels.desktop"
  />
</template>
