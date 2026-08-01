<script setup lang="ts">
import type { SettingsTab } from './runtimePageStateModel'
import type { useSettingsPageState } from './useSettingsPageState'
import type { SessionExecutionResource } from '../lib/agenaApi'

import SettingsConfigurationPageContent from './SettingsConfigurationPageContent.vue'
import SettingsMemoryPageContent from './SettingsMemoryPageContent.vue'
import SettingsPermissionsPageContent from './SettingsPermissionsPageContent.vue'
import SettingsPluginsPageContent from './SettingsPluginsPageContent.vue'
import SettingsProvidersPageContent from './SettingsProvidersPageContent.vue'

const props = defineProps<{
  activeTab: SettingsTab
  loading: boolean
  load: () => Promise<void>
  panels: ReturnType<typeof useSettingsPageState>['panels']
  permissionScope: 'global' | 'session'
  selectedSessionId: number | null
  sessionExecution: SessionExecutionResource | null
}>()
</script>

<template>
  <SettingsProvidersPageContent v-if="props.activeTab === 'providers'" :providers="props.panels.providers" />

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
    :permission-scope="props.permissionScope"
    :selected-session-id="props.selectedSessionId"
    :session-execution="props.sessionExecution"
  />
</template>
