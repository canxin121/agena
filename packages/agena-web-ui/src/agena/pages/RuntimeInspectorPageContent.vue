<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeInspectorPanel from './RuntimeInspectorPanel.vue'

const props = defineProps<{
  inspectors: ReturnType<typeof useRuntimeSectionPageState>['inspectors']
  kind: 'mcp' | 'lsp'
}>()
</script>

<template>
  <RuntimeInspectorPanel
    :kind="props.kind"
    :runtime="props.inspectors.runtime"
    :filtered-mcp-servers="props.inspectors.filteredMcpServers"
    :filtered-lsp-servers="props.inspectors.filteredLspServers"
    :mcp-query="props.inspectors.mcpQuery"
    :lsp-query="props.inspectors.lspQuery"
    :open-runtime-config-root="props.inspectors.openRuntimeConfigRoot"
    :open-workspace-shortcut="props.inspectors.openWorkspaceShortcut"
    :open-workspace-path="props.inspectors.openWorkspacePath"
    @update:mcp-query="props.inspectors.mcpQuery = $event"
    @update:lsp-query="props.inspectors.lspQuery = $event"
  />
</template>
