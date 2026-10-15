<script setup lang="ts">
defineProps<{ modelValue: boolean; label: string; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()
</script>

<template>
  <label
    class="relative inline-flex shrink-0 cursor-pointer items-center py-1"
    :class="disabled ? 'cursor-not-allowed opacity-50' : ''"
  >
    <input
      type="checkbox"
      role="switch"
      class="peer absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
      :checked="modelValue"
      :aria-label="label"
      :disabled="disabled"
      @change="emit('update:modelValue', ($event.target as HTMLInputElement).checked)"
    />
    <span
      aria-hidden="true"
      class="pointer-events-none h-5 w-9 rounded-full border border-border bg-muted transition-colors peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background"
    />
    <span
      aria-hidden="true"
      class="pointer-events-none absolute left-0.5 h-4 w-4 rounded-full bg-background shadow-sm transition-transform peer-checked:translate-x-4"
    />
  </label>
</template>
