<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import SettingsPluginsPanel from './SettingsPluginsPanel.vue'

const props = defineProps<{
  plugins: ReturnType<typeof useSettingsPageState>['panels']['plugins']
}>()
</script>

<template>
  <SettingsPluginsPanel
    :action-error="props.plugins.actionError.value"
    :action-message="props.plugins.actionMessage.value"
    :default-mode="props.plugins.defaultMode.value"
    :load="props.plugins.load"
    :mode-options="props.plugins.modeOptions"
    :plugin-entries="props.plugins.pluginEntries.value"
    :plugin-entry-summary="props.plugins.pluginEntrySummary"
    :summary-facts="props.plugins.summaryFacts.value"
    :set-default-tool-description-mode="props.plugins.setDefaultToolDescriptionMode"
    :toggle-plugin-entry-disabled="props.plugins.togglePluginEntryDisabled"
  />
</template>
