<script setup lang="ts">
import { onMounted } from 'vue'

import type { useSettingsPageState } from './useSettingsPageState'
import SettingsConfigurationPanel from './SettingsConfigurationPanel.vue'

const props = defineProps<{
  configuration: ReturnType<typeof useSettingsPageState>['panels']['configuration']
}>()

onMounted(() => {
  if (!props.configuration.fileSettings.value && !props.configuration.loading.value) {
    void props.configuration.load(false)
  }
})
</script>

<template>
  <SettingsConfigurationPanel :configuration="props.configuration" />
</template>
