<script setup lang="ts">
import type { Component } from 'vue'
import Button from '@/components/ui/Button.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import PermissionSettingsCard from './PermissionSettingsCard.vue'

defineProps<{
  title: string
  description?: string
  icon?: Component
  count: number
  visibleCount: number
  query: string
  selectedCount: number
  allSelected: boolean
  editable: boolean
  disabled?: boolean
}>()
defineEmits<{
  'update:query': [value: string]
  'toggle-all': []
  'clear-selection': []
}>()
</script>

<template>
  <PermissionSettingsCard :title="title" :description="description" :icon="icon" :count="count">
    <div v-if="count" class="mb-3 flex flex-wrap items-center gap-3">
      <SearchInput
        :model-value="query"
        class="min-w-0 flex-1 basis-48"
        :placeholder="$st('Search rules…')"
        :input-aria-label="$st('Search rules…')"
        :input-title="title"
        :show-search-button="false"
        :clear-aria-label="$st('Clear search')"
        :clear-title="$st('Clear search')"
        @update:model-value="$emit('update:query', $event)"
      />
      <label v-if="editable && visibleCount" class="flex items-center gap-2 text-xs text-muted-foreground">
        <input
          type="checkbox"
          class="accent-primary"
          :checked="allSelected"
          :disabled="disabled"
          :indeterminate="selectedCount > 0 && !allSelected"
          @change="$emit('toggle-all')"
        />
        {{ $st('Select all') }}
      </label>
    </div>
    <div
      v-if="editable && selectedCount"
      class="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-primary/20 bg-primary/5 p-2.5"
    >
      <span class="text-xs font-medium">{{ $st('{count} selected', { count: selectedCount }) }}</span>
      <Button variant="ghost" size="sm" :disabled="disabled" @click="$emit('clear-selection')">{{
        $st('Select none')
      }}</Button>
      <div class="flex flex-1 flex-wrap items-center justify-end gap-2"><slot name="bulk" /></div>
    </div>
    <div v-if="visibleCount" class="grid min-w-0 gap-2"><slot /></div>
    <div v-else-if="count" class="rounded-lg bg-muted/20 px-4 py-5 text-center">
      <p class="text-sm text-muted-foreground">{{ $st('No matching rules.') }}</p>
      <Button variant="ghost" size="sm" class="mt-1" @click="$emit('update:query', '')">{{
        $st('Clear search')
      }}</Button>
    </div>
    <p v-else class="py-3 text-sm text-muted-foreground">
      {{ $st('No exceptions in this scope. Default policies apply.') }}
    </p>
    <fieldset v-if="editable" :disabled="disabled" class="mt-3 min-w-0 border-t border-border/50 pt-3">
      <slot name="create" />
    </fieldset>
  </PermissionSettingsCard>
</template>
