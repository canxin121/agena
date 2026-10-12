<script setup lang="ts">
import { RiArrowDownSLine, RiArrowRightSLine, RiLoader4Line } from '@remixicon/vue'

defineProps<{ title: string; expanded: boolean; summary?: string; busy?: boolean; docked?: boolean }>()
defineEmits<{ (event: 'update:expanded', value: boolean): void }>()
</script>

<template>
  <section
    class="min-w-0 rounded-lg"
    :class="
      docked
        ? 'relative my-0 shrink-0 border border-emerald-500/25 bg-emerald-500/10 text-emerald-100/90'
        : 'my-1.5 w-full border border-border/60 bg-secondary/10'
    "
    :aria-label="title"
    data-transcript-chrome="true"
    data-session-section
  >
    <header class="flex min-h-7 max-w-full items-center gap-x-1 px-1">
      <button
        type="button"
        class="flex min-h-7 max-w-full items-center gap-1.5 rounded px-1.5 text-left text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="docked ? 'hover:bg-emerald-500/15' : 'hover:bg-secondary/50'"
        :aria-expanded="expanded"
        @click="$emit('update:expanded', !expanded)"
      >
        <component :is="expanded ? RiArrowDownSLine : RiArrowRightSLine" class="h-4 w-4 shrink-0" />
        <span class="shrink-0 font-semibold">{{ title }}</span>
        <span
          v-if="summary"
          class="max-w-44 truncate"
          :class="docked ? 'text-emerald-100/70' : 'text-muted-foreground'"
          :title="summary"
          >{{ summary }}</span
        >
      </button>
      <slot name="actions" />
      <RiLoader4Line v-if="busy" class="h-3.5 w-3.5 shrink-0 animate-spin" />
    </header>
    <div
      v-if="expanded"
      class="min-w-0 border-t border-border/50 bg-background px-3 py-2 shadow-xl"
      :class="
        docked
          ? 'absolute bottom-[calc(100%+0.5rem)] left-0 z-50 flex max-h-[min(70dvh,40rem)] w-[min(92vw,42rem)] flex-col overflow-auto rounded-xl border'
          : ''
      "
    >
      <slot />
    </div>
  </section>
</template>
