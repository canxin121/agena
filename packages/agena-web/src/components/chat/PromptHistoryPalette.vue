<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { RiHistoryLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ListRowButton from '@/components/ui/ListRowButton.vue'

const props = defineProps<{
  open: boolean
  autoFocus: boolean
  query: string
  entries: string[]
  activeIndex: number
}>()

const emit = defineEmits<{
  (e: 'update:activeIndex', value: number): void
  (e: 'update:query', value: string): void
  (e: 'keydown', event: KeyboardEvent): void
  (e: 'select', text: string): void
}>()

const rootEl = ref<HTMLDivElement | null>(null)
const searchInput = ref<HTMLInputElement | null>(null)
const { t } = useI18n()

function containsTarget(target: Node | null): boolean {
  if (!target) return false
  return Boolean(rootEl.value && rootEl.value.contains(target))
}

function focusSearch() {
  void nextTick(() => searchInput.value?.focus())
}

defineExpose({ rootEl, containsTarget, focusSearch })

watch(
  () => [props.open, props.autoFocus] as const,
  ([open, autoFocus]) => {
    if (open && autoFocus) focusSearch()
  },
)

function setIndex(index: number) {
  emit('update:activeIndex', index)
}

function preview(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}
</script>

<template>
  <div
    v-if="open"
    ref="rootEl"
    data-prompt-history-palette="true"
    class="absolute bottom-full mb-2 left-0 w-full max-w-[560px] rounded-xl border border-border bg-background/95 shadow-lg z-20"
  >
    <div class="flex items-center gap-2 border-b border-border/60 px-3 py-2">
      <RiHistoryLine class="h-4 w-4 shrink-0 text-primary" />
      <span class="font-mono text-sm text-primary">{{ t('chat.composer.promptHistory.label') }}</span>
      <input
        ref="searchInput"
        :value="query"
        type="text"
        class="h-7 min-w-0 flex-1 border-0 bg-transparent font-mono text-sm outline-none placeholder:text-muted-foreground"
        :placeholder="t('chat.composer.promptHistory.search')"
        autocomplete="off"
        spellcheck="false"
        :aria-label="t('chat.composer.promptHistory.search')"
        @input="$emit('update:query', ($event.target as HTMLInputElement).value)"
        @keydown="$emit('keydown', $event)"
      />
    </div>

    <div class="max-h-64 overflow-auto px-2 py-2">
      <div v-if="entries.length === 0" class="px-3 py-2 text-sm text-muted-foreground">
        {{ query.trim() ? t('chat.composer.promptHistory.noMatches') : t('chat.composer.promptHistory.empty') }}
      </div>
      <div v-else class="space-y-1">
        <ListRowButton
          v-for="(entry, index) in entries"
          :key="entry"
          :active="index === activeIndex"
          size="sm"
          :aria-selected="index === activeIndex"
          @click="$emit('select', entry)"
          @mouseenter="setIndex(index)"
        >
          <span class="w-8 shrink-0 font-mono text-[11px] text-muted-foreground">#{{ index + 1 }}</span>
          <span class="min-w-0 flex-1 truncate text-left text-sm" :title="entry">{{ preview(entry) }}</span>
        </ListRowButton>
      </div>
    </div>

    <div class="border-t border-border/60 px-3 py-1.5 text-[11px] text-muted-foreground">
      {{ t('chat.composer.promptHistory.hint') }}
    </div>
  </div>
</template>
