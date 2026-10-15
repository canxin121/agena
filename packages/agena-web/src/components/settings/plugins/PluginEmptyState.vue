<script setup lang="ts">
import type { Component } from 'vue'
import { RiPlugLine } from '@remixicon/vue'

defineProps<{ title: string; description?: string; icon?: Component }>()
</script>

<template>
  <div class="flex flex-col items-center px-5 py-12 text-center" role="status">
    <div class="mb-4 flex h-11 w-11 items-center justify-center rounded-xl bg-muted/50 text-muted-foreground">
      <component :is="icon || RiPlugLine" class="h-5 w-5" aria-hidden="true" />
    </div>
    <p class="text-sm font-medium">{{ title }}</p>
    <p v-if="description" class="mt-1.5 max-w-sm text-xs leading-relaxed text-muted-foreground">{{ description }}</p>
    <div v-if="$slots.default" class="mt-4"><slot /></div>
  </div>
</template>
