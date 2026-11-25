<script setup lang="ts">
import { RiHistoryLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import ComposerPalette from './ComposerPalette.vue'

defineProps<{
  open: boolean
  autoFocus: boolean
  query: string
  entries: string[]
  activeIndex: number
  listId?: string
}>()
const emit = defineEmits<{
  'update:activeIndex': [index: number]
  'update:query': [query: string]
  keydown: [event: KeyboardEvent]
  select: [text: string]
  dismiss: []
}>()
const { t } = useI18n()

function preview(text: string) {
  return text.slice(0, 600).replace(/\s+/g, ' ').trim()
}
</script>

<template>
  <ComposerPalette
    kind="history"
    :open="open"
    :auto-focus="autoFocus"
    :query="query"
    :items="entries"
    :item-key="(entry) => entry"
    :active-index="activeIndex"
    :list-id="listId"
    :search-label="t('chat.composer.promptHistory.search')"
    :list-label="t('chat.composer.promptHistory.label')"
    :empty-text="query.trim() ? t('chat.composer.promptHistory.noMatches') : t('chat.composer.promptHistory.empty')"
    :hint="t('chat.composer.promptHistory.hint')"
    @update:query="emit('update:query', $event)"
    @update:active-index="emit('update:activeIndex', $event)"
    @keydown="emit('keydown', $event)"
    @select="emit('select', $event)"
    @dismiss="emit('dismiss')"
  >
    <template #icon><RiHistoryLine class="size-4 shrink-0 text-primary" aria-hidden="true" /></template>
    <template #item="{ item }">
      <span class="min-w-0 flex-1 truncate text-sm" :title="item">{{ preview(item) }}</span>
    </template>
  </ComposerPalette>
</template>
