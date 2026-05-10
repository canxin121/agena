<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeInspectorPageContent from './RuntimeInspectorPageContent.vue'
import RuntimeOperatorPageContent from './RuntimeOperatorPageContent.vue'
import RuntimeOverviewPageContent from './RuntimeOverviewPageContent.vue'
import RuntimeSkillsPageContent from './RuntimeSkillsPageContent.vue'
import RuntimeWorkflowPageContent from './RuntimeWorkflowPageContent.vue'

const props = defineProps<{
  activeTab: ReturnType<typeof useRuntimeSectionPageState>['activeTab']['value']
  formatProviderModel: ReturnType<typeof useRuntimeSectionPageState>['overview']['formatProviderModel']
  panels: ReturnType<typeof useRuntimeSectionPageState>['panels']
}>()
</script>

<template>
  <RuntimeOverviewPageContent
    v-if="props.activeTab === 'overview'"
    :overview="props.panels.overview"
    :format-provider-model="props.formatProviderModel"
  />

  <RuntimeWorkflowPageContent
    v-else-if="props.activeTab === 'workflow'"
    :workflow="props.panels.workflow"
  />

  <RuntimeInspectorPageContent
    v-else-if="props.activeTab === 'mcp'"
    kind="mcp"
    :inspectors="props.panels.mcp"
  />

  <RuntimeInspectorPageContent
    v-else-if="props.activeTab === 'lsp'"
    kind="lsp"
    :inspectors="props.panels.lsp"
  />

  <RuntimeSkillsPageContent
    v-else-if="props.activeTab === 'skills'"
    :skills="props.panels.skills"
  />

  <RuntimeOperatorPageContent v-else :operator="props.panels.operator" />
</template>
