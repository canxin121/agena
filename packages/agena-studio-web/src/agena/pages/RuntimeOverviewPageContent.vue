<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeOverviewPanel from './RuntimeOverviewPanel.vue'

const props = defineProps<{
  formatProviderModel: ReturnType<typeof useRuntimeSectionPageState>['overview']['formatProviderModel']
  load: ReturnType<typeof useRuntimeSectionPageState>['load']
  overview: ReturnType<typeof useRuntimeSectionPageState>['overview']
}>()
</script>

<template>
  <RuntimeOverviewPanel
    :catalog-entries="props.overview.catalogEntries"
    :operator-cards="props.overview.operatorCards"
    :runtime-snapshot-facts="props.overview.runtimeSnapshotFacts"
    :runtime="props.overview.runtime"
    :providers="props.overview.providers"
    :provider-models="props.overview.providerModels"
    :session-cache-facts="props.overview.sessionCacheFacts"
    :format-provider-model="props.formatProviderModel"
    :load="props.load"
  />
</template>
