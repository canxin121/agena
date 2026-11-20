<script setup lang="ts">
import { RiInformationLine } from '@remixicon/vue'
defineProps<{ text: string }>()
</script>

<template>
  <details class="permission-help relative shrink-0 text-[13px] font-normal">
    <summary
      class="flex h-8 w-8 cursor-pointer list-none items-center justify-center rounded text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      :aria-label="text"
    >
      <RiInformationLine class="h-4 w-4" aria-hidden="true" />
    </summary>
    <div
      class="absolute end-0 top-full z-40 w-64 max-w-[calc(100vw-3rem)] rounded-md border border-border bg-popover p-3 leading-relaxed text-popover-foreground shadow-md"
    >
      {{ text }}
    </div>
  </details>
</template>

<style scoped>
summary::-webkit-details-marker {
  display: none;
}
</style>
