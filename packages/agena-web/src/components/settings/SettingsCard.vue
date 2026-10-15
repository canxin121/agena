<script setup lang="ts">
import type { Component } from 'vue'

defineProps<{ title: string; description?: string; count?: number; icon?: Component }>()
</script>

<template>
  <section class="min-w-0 rounded-xl border border-border/70 bg-card/30" data-settings-card>
    <header class="flex flex-wrap items-center gap-3 rounded-t-xl border-b border-border/60 bg-muted/20 px-4 py-3.5">
      <component :is="icon" v-if="icon" class="h-4.5 w-4.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div class="min-w-0 flex-1 basis-40">
        <h2 class="flex items-center gap-2 text-[15px] font-semibold">
          {{ title }}
          <span
            v-if="count !== undefined"
            class="rounded-md bg-muted/70 px-1.5 py-0.5 text-[11px] font-medium tabular-nums text-muted-foreground"
            >{{ count }}</span
          >
        </h2>
        <p v-if="description" class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ description }}</p>
      </div>
      <slot name="actions" />
    </header>
    <div class="min-w-0 p-3 sm:p-4"><slot /></div>
  </section>
</template>
