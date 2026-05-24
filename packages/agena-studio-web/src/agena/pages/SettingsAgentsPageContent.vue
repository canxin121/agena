<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import SettingsAgentsPanel from './SettingsAgentsPanel.vue'

const props = defineProps<{
  agents: ReturnType<typeof useSettingsPageState>['panels']['agents']
}>()
</script>

<template>
  <SettingsAgentsPanel
    :action-error="props.agents.actionError.value"
    :action-message="props.agents.actionMessage.value"
    :agent-cards="props.agents.agentCards"
    :load="props.agents.load"
    :summary-facts="props.agents.summaryFacts"
    :set-default-agent="props.agents.setDefaultAgent"
  />
</template>
