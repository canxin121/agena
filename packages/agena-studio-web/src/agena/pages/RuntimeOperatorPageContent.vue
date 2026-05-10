<script setup lang="ts">
import type { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

import RuntimeOperatorPanel from './RuntimeOperatorPanel.vue'

const props = defineProps<{
  operator: ReturnType<typeof useRuntimeSectionPageState>['operator']
}>()
</script>

<template>
  <RuntimeOperatorPanel :runtime="props.operator.runtime" />
</template>
