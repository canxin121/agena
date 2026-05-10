<script setup lang="ts">
import type { usePluginsPageState } from './usePluginsPageState'

import PluginsInstalledPanel from './PluginsInstalledPanel.vue'

const props = defineProps<{
  installed: ReturnType<typeof usePluginsPageState>['panels']['installed']
}>()
</script>

<template>
  <PluginsInstalledPanel
    :plugins="props.installed.plugins"
    :selected-plugin="props.installed.selectedPlugin"
    :plugin-logs="props.installed.pluginLogs"
    :plugin-loading="props.installed.pluginLoading"
    :load-plugin-details="props.installed.loadPluginDetails"
    :open-plugin-manifest-in-workspace="props.installed.openPluginManifestInWorkspace"
    :open-plugin-logs-workspace-path="props.installed.openPluginLogsWorkspacePath"
  />
</template>
