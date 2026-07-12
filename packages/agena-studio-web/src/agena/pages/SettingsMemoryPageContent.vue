<script setup lang="ts">
import { onMounted } from 'vue'
import { useRoute } from 'vue-router'

import SettingsMemoryPanel from './SettingsMemoryPanel.vue'
import type { useSettingsPageState } from './useSettingsPageState'

const props = defineProps<{
  memory: ReturnType<typeof useSettingsPageState>['panels']['memory']
}>()
const route = useRoute()

onMounted(() => {
  const preferredName = typeof route.query.memory === 'string' ? route.query.memory : undefined
  if (!props.memory.memories.value.length && !props.memory.loading.value) void props.memory.load(preferredName)
})
</script>

<template>
  <SettingsMemoryPanel :memory="props.memory" />
</template>
