<script setup lang="ts">
import type { usePluginsPageState } from './usePluginsPageState'

import PluginsInstalledPanel from './PluginsInstalledPanel.vue'

const props = defineProps<{
  installed: ReturnType<typeof usePluginsPageState>['panels']['installed']
}>()
</script>

<template>
  <PluginsInstalledPanel
    :can-toggle-plugin-config="props.installed.canTogglePluginConfig.value"
    :selected-plugin="props.installed.selectedPlugin"
    :plugin-logs="props.installed.pluginLogs"
    :plugin-loading="props.installed.pluginLoading"
    :plugin-ui-presentation="props.installed.pluginUiPresentation.value"
    :plugins="props.installed.plugins"
    :load-plugin-details="props.installed.loadPluginDetails"
    :open-plugin-manifest-in-workspace="props.installed.openPluginManifestInWorkspace"
    :open-plugin-logs-workspace-path="props.installed.openPluginLogsWorkspacePath"
    :set-selected-plugin-disabled="props.installed.setSelectedPluginDisabled"
  />
</template>
