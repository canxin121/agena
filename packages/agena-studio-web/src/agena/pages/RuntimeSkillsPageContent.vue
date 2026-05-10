<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeSkillsPanel from './RuntimeSkillsPanel.vue'

const props = defineProps<{
  skills: ReturnType<typeof useRuntimeSectionPageState>['skills']
}>()
</script>

<template>
  <RuntimeSkillsPanel
    :runtime-skill-query="props.skills.runtimeSkillQuery"
    :catalog-sections="props.skills.catalogSections"
    :open-workspace-shortcut="props.skills.openWorkspaceShortcut"
    :open-runtime-config-root="props.skills.openRuntimeConfigRoot"
    :open-plugin-logs-workspace-path="props.skills.openPluginLogsWorkspacePath"
    :open-runtime-entry-in-chat="props.skills.openRuntimeEntryInChat"
    :open-runtime-entry-source="props.skills.openRuntimeEntrySource"
    @update:runtime-skill-query="props.skills.runtimeSkillQuery = $event"
  />
</template>
