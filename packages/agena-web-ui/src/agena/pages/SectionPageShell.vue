<script setup lang="ts">
const props = defineProps<{
  actionError: string
  actionMessage: string
  loading: boolean
  pageDescription: string
  pageTitle: string
  refreshLabel?: string
  showRefresh?: boolean
}>()

const emit = defineEmits<{
  refresh: []
}>()
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">{{ props.pageTitle }}</h1>
        <p class="page-description">{{ props.pageDescription }}</p>
      </div>
      <div class="button-row">
        <slot name="header-actions">
          <button
            v-if="props.showRefresh !== false"
            class="button ghost"
            :disabled="props.loading"
            @click="emit('refresh')"
          >
            {{ props.refreshLabel || 'Refresh' }}
          </button>
        </slot>
      </div>
    </header>

    <div v-if="props.actionError" class="notice">{{ props.actionError }}</div>
    <div v-else-if="props.actionMessage" class="notice">{{ props.actionMessage }}</div>

    <slot />
  </section>
</template>
