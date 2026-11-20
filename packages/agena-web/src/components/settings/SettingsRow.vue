<script setup lang="ts">
defineProps<{ title: string; description?: string; toggle?: boolean }>()
</script>

<template>
  <div
    class="settings-field-row border-b border-border/50 py-1.5 first:pt-0 last:border-b-0 last:pb-0"
    :class="{ 'settings-field-toggle': toggle }"
  >
    <div class="min-w-0">
      <div class="break-words text-[13px] font-medium">{{ title }}</div>
      <p v-if="description" class="mt-0.5 break-words text-xs leading-snug text-muted-foreground">{{ description }}</p>
    </div>
    <div class="min-w-0" :class="toggle ? 'flex items-center justify-end' : ''"><slot /></div>
  </div>
</template>
