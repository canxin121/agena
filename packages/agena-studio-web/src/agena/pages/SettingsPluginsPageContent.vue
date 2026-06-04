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
    :load="props.plugins.load"
    :plugin-has-overrides="props.plugins.pluginHasOverrides"
    :plugins="props.plugins.plugins.value"
    :plugin-entry-summary="props.plugins.pluginEntrySummary"
    :plugin-prompt-source-summary="props.plugins.pluginPromptSourceSummary"
    :plugin-ui-source-summary="props.plugins.pluginUiSourceSummary"
    :primary-plugin-text="props.plugins.primaryPluginText"
    :primary-tool-text="props.plugins.primaryToolText"
    :prompt-mode-label="props.plugins.toolPromptModeLabel"
    :prompt-override-label="props.plugins.promptOverrideLabel"
    :prompt-override-options="props.plugins.promptOverrideOptions"
    :secondary-plugin-text="props.plugins.secondaryPluginText"
    :secondary-tool-text="props.plugins.secondaryToolText"
    :set-plugin-prompt-override="props.plugins.setPluginPromptOverride"
    :set-plugin-ui-display-override="props.plugins.setPluginUiDisplayOverride"
    :set-tool-prompt-override="props.plugins.setToolPromptOverride"
    :set-tool-ui-display-override="props.plugins.setToolUiDisplayOverride"
    :summary-facts="props.plugins.summaryFacts.value"
    :toggle-plugin-entry-disabled="props.plugins.togglePluginEntryDisabled"
    :tool-prompt-source-summary="props.plugins.toolPromptSourceSummary"
    :tool-ui-source-summary="props.plugins.toolUiSourceSummary"
    :ui-display-mode-label="props.plugins.uiDisplayModeLabel"
    :ui-display-override-label="props.plugins.uiDisplayOverrideLabel"
    :ui-display-override-options="props.plugins.uiDisplayOverrideOptions"
  />
</template>
