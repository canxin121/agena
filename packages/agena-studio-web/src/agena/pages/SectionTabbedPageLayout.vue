<script setup lang="ts">
import SectionPageShell from './SectionPageShell.vue'
import SectionTabBar from './SectionTabBar.vue'

const props = defineProps<{
  activeTab: string
  actionError: string
  actionMessage: string
  loading: boolean
  pageDescription: string
  pageTitle: string
  tabs: ReadonlyArray<{
    id: string
    label: string
  }>
}>()

const emit = defineEmits<{
  refresh: []
  'update:activeTab': [value: string]
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
    <SectionTabBar
      :active-tab="props.activeTab"
      :tabs="props.tabs"
      @update:active-tab="emit('update:activeTab', $event)"
    />

    <slot />
  </SectionPageShell>
</template>
