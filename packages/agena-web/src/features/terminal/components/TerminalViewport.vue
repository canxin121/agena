<script setup lang="ts">
import { ref } from 'vue'
import { terminalBackground } from '../lib/useTerminalViewport'

const element = ref<HTMLDivElement | null>(null)
defineExpose({ element })
</script>

<template>
  <div class="relative min-h-0 min-w-0 flex-1 overflow-hidden" :style="{ backgroundColor: terminalBackground }">
    <div ref="element" dir="ltr" class="absolute inset-0" />
  </div>
</template>
