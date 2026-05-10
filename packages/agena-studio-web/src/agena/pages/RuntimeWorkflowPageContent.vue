<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeWorkflowPanel from './RuntimeWorkflowPanel.vue'

const props = defineProps<{
  workflow: ReturnType<typeof useRuntimeSectionPageState>['workflow']
}>()
</script>

<template>
  <RuntimeWorkflowPanel
    :selected-workspace-id="props.workflow.selectedWorkspaceId"
    :selected-session-id="props.workflow.selectedSessionId"
    :workspaces="props.workflow.workspaces"
    :sessions="props.workflow.sessions"
    :execution-facts="props.workflow.executionFacts"
    :workflow-loading="props.workflow.workflowLoading"
    :session-execution="props.workflow.sessionExecution"
    :timeline-summaries="props.workflow.timelineSummaries"
    :open-selected-session-in-chat="props.workflow.openSelectedSessionInChat"
    :select-workspace="props.workflow.selectWorkspace"
    :select-session="props.workflow.selectSession"
    :approve-permission="props.workflow.approvePermission"
  />
</template>
