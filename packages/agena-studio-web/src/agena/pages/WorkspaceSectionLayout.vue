<script setup lang="ts">
import SectionPageShell from './SectionPageShell.vue'

const props = defineProps<{
  actionError: string
  actionMessage: string
  loading: boolean
  pageDescription: string
  pageTitle: string
}>()

const emit = defineEmits<{
  refresh: []
  root: []
}>()
</script>

<template>
  <SectionPageShell
    :action-error="props.actionError"
    :action-message="props.actionMessage"
    :loading="props.loading"
    :page-description="props.pageDescription"
    :page-title="props.pageTitle"
    @refresh="emit('refresh')"
  >
    <template #header-actions>
      <button class="button ghost" :disabled="props.loading" @click="emit('refresh')">Refresh</button>
      <button class="button" :disabled="props.loading" @click="emit('root')">Root</button>
    </template>

    <slot />
  </SectionPageShell>
</template>
