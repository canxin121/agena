<script setup lang="ts">
import { RiArrowDownSLine, RiArrowRightSLine, RiLoader4Line } from '@remixicon/vue'

defineProps<{ title: string; expanded: boolean; summary?: string; busy?: boolean; docked?: boolean }>()
defineEmits<{ (event: 'update:expanded', value: boolean): void }>()
</script>

<template>
  <section
    class="w-full min-w-0 rounded-lg border border-border/60 bg-secondary/10"
    :class="docked ? 'my-0 flex min-h-0 flex-col overflow-hidden' : 'my-1.5'"
    :aria-label="title"
    data-transcript-chrome="true"
    data-session-section
  >
    <header class="flex min-h-7 shrink-0 flex-wrap items-center gap-x-2 px-2">
      <button
        type="button"
        class="flex min-h-7 shrink-0 items-center gap-2 rounded px-1 text-left text-xs hover:bg-secondary/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :aria-expanded="expanded"
        @click="$emit('update:expanded', !expanded)"
      >
        <component :is="expanded ? RiArrowDownSLine : RiArrowRightSLine" class="h-4 w-4 shrink-0" />
        <span class="shrink-0 font-semibold">{{ title }}</span>
      </button>
      <slot name="actions" />
      <span v-if="summary" class="min-w-0 flex-1 truncate text-xs text-muted-foreground" :title="summary">{{
        summary
      }}</span>
      <RiLoader4Line v-if="busy" class="h-3.5 w-3.5 shrink-0 animate-spin" />
    </header>
    <div
      v-if="expanded"
      class="min-w-0 border-t border-border/50 px-3 py-1.5"
      :class="docked ? 'flex min-h-0 flex-col' : ''"
    >
      <slot />
    </div>
  </section>
</template>
