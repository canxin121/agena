<script setup lang="ts">
import { onMounted } from 'vue'
import type { useSettingsPageState } from './useSettingsPageState'

import SettingsAgentsPanel from './SettingsAgentsPanel.vue'

const props = defineProps<{
  agents: ReturnType<typeof useSettingsPageState>['panels']['agents']
}>()

onMounted(() => {
  void props.agents.loadConfigAgents()
})
</script>

<template>
  <SettingsAgentsPanel :agents="props.agents" />
</template>
