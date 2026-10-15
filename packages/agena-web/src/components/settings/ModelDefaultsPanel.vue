<script setup lang="ts">
import { ref } from 'vue'
import ModelSelectionSettings from './ModelSelectionSettings.vue'
withDefaults(defineProps<{ scope?: string }>(), { scope: 'effective' })
const editor = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({
  refresh: async () => {
    await editor.value?.refresh()
  },
})
</script>

<template>
  <ModelSelectionSettings ref="editor" purpose="default" :scope="scope" />
</template>
