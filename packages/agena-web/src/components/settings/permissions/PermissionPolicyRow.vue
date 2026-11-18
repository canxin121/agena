<script setup lang="ts">
defineProps<{ label: string; value: string; editable: boolean; hint?: string; origin?: string }>()
</script>

<template>
  <div class="permission-policy-row">
    <span class="min-w-0 text-[13px] leading-snug" :title="hint"
      >{{ label }}<span v-if="origin" class="block text-[13px] text-muted-foreground">{{ origin }}</span></span
    >
    <div v-if="editable" class="min-w-0"><slot /></div>
    <span v-else class="permission-policy-value text-[13px] font-medium" data-permission-value>{{ value }}</span>
  </div>
</template>

<style scoped>
.permission-policy-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(7rem, 52%);
  align-items: center;
  gap: 0.5rem;
  min-height: 36px;
}
.permission-policy-value {
  padding: 0.25rem 0.5rem;
  border-radius: 0.25rem;
  background: color-mix(in oklch, var(--color-muted) 45%, transparent);
  text-align: end;
}
</style>
