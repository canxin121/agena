<script setup lang="ts">
import type { Component } from 'vue'

defineProps<{ title: string; description?: string; count?: number; icon?: Component; heading?: 'h2' | 'h3' }>()
</script>

<template>
  <section class="settings-card" data-settings-card>
    <header class="settings-card-heading">
      <component :is="icon" v-if="icon" class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div class="min-w-0 flex-1 basis-36">
        <component :is="heading || 'h2'" class="flex flex-wrap items-center gap-1.5 text-sm font-semibold">
          {{ title }}
          <span
            v-if="count !== undefined"
            class="rounded-md bg-muted/70 px-1.5 py-0.5 text-[11px] font-medium tabular-nums text-muted-foreground"
            >{{ count }}</span
          >
        </component>
        <p v-if="description" class="mt-0.5 text-xs leading-snug text-muted-foreground">{{ description }}</p>
      </div>
      <slot name="actions" />
    </header>
    <div class="settings-card-body"><slot /></div>
  </section>
</template>

<style scoped>
@reference '../../style.css';
.settings-card {
  @apply rounded-lg border border-border/70 bg-card/30;
  container: settings-card / inline-size;
  min-width: 0;
}
.settings-card-heading {
  @apply rounded-t-lg border-b border-border/60 bg-muted/20;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem 0.625rem;
  min-width: 0;
  min-height: 2.375rem;
  padding: 0.375rem 0.75rem;
  overflow-wrap: anywhere;
}
.settings-card-body {
  min-width: 0;
  padding: 0.625rem 0.75rem;
}
</style>
