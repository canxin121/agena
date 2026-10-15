<script setup lang="ts">
defineProps<{ title: string; description?: string; toggle?: boolean }>()
</script>

<template>
  <div
    class="grid min-w-0 gap-3 border-b border-border/50 py-3.5 first:pt-0 last:border-b-0 last:pb-0"
    :class="
      toggle
        ? 'grid-cols-[minmax(0,1fr)_auto] items-center'
        : 'sm:grid-cols-[minmax(0,1fr)_minmax(12rem,0.65fr)] sm:items-center'
    "
  >
    <div class="min-w-0">
      <div class="break-words text-sm font-medium">{{ title }}</div>
      <p v-if="description" class="mt-1 break-words text-xs leading-relaxed text-muted-foreground">{{ description }}</p>
    </div>
    <div class="min-w-0" :class="toggle ? 'flex items-center justify-end' : ''"><slot /></div>
  </div>
</template>
