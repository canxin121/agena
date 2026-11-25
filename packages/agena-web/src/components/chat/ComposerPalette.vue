<script setup lang="ts" generic="Item">
import { computed, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiLoader4Line } from '@remixicon/vue'
import ListRowButton from '@/components/ui/ListRowButton.vue'
import { useComposerPaletteFocus } from '@/composables/useComposerPaletteFocus'

const props = defineProps<{
  kind: 'command' | 'history'
  open: boolean
  autoFocus: boolean
  loading?: boolean
  query: string
  items: readonly Item[]
  itemKey: (item: Item) => string | number
  activeIndex: number
  searchLabel: string
  listLabel: string
  emptyText: string
  hint: string
  listId?: string
}>()
const emit = defineEmits<{
  'update:activeIndex': [index: number]
  'update:query': [query: string]
  keydown: [event: KeyboardEvent]
  select: [item: Item]
  dismiss: []
}>()
const { t } = useI18n()
const root = ref<HTMLDivElement | null>(null)
const search = ref<HTMLInputElement | null>(null)
const id = useId()
const resultsId = computed(() => props.listId || `${props.kind}-${id}`)
const activeId = computed(() =>
  props.items[props.activeIndex] !== undefined ? `${resultsId.value}-${props.activeIndex}` : undefined,
)

useComposerPaletteFocus(
  root,
  search,
  () => props.open,
  () => props.autoFocus,
  () => emit('dismiss'),
)

watch(
  () => [root.value, props.activeIndex, props.items] as const,
  () => {
    root.value
      ?.querySelector('[role="option"][aria-selected="true"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  },
  { flush: 'post' },
)
</script>

<template>
  <div
    v-if="open"
    ref="root"
    data-keyboard-overlay="true"
    :data-command-palette="kind === 'command' ? 'true' : undefined"
    :data-prompt-history-palette="kind === 'history' ? 'true' : undefined"
    class="oc-overlay-panel absolute inset-x-0 bottom-full z-20 mb-2 w-full max-w-[35rem] overflow-hidden"
  >
    <div class="flex items-center gap-2 border-b border-border/60 px-3 py-2 focus-within:bg-secondary/20">
      <slot name="icon" />
      <input
        ref="search"
        :value="query"
        type="text"
        class="h-7 min-w-0 flex-1 border-0 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        :class="kind === 'command' && 'font-mono'"
        :placeholder="searchLabel"
        autocomplete="off"
        :spellcheck="false"
        :aria-label="searchLabel"
        role="combobox"
        aria-autocomplete="list"
        aria-expanded="true"
        :aria-controls="resultsId"
        :aria-activedescendant="activeId"
        @input="emit('update:query', ($event.target as HTMLInputElement).value)"
        @keydown="emit('keydown', $event)"
      />
    </div>
    <div
      :id="resultsId"
      role="listbox"
      tabindex="-1"
      :aria-label="listLabel"
      :aria-busy="loading || undefined"
      class="max-h-[min(16rem,45dvh)] overflow-y-auto overscroll-contain p-2"
    >
      <div
        v-if="loading && !items.length"
        role="status"
        :aria-label="t('common.loading')"
        class="grid place-items-center py-4 text-muted-foreground"
      >
        <RiLoader4Line class="size-4 animate-spin" aria-hidden="true" />
      </div>
      <p v-else-if="!items.length" class="px-2 py-2 text-sm text-muted-foreground">{{ emptyText }}</p>
      <div v-else class="space-y-1">
        <ListRowButton
          v-for="(item, index) in items"
          :id="`${resultsId}-${index}`"
          :key="itemKey(item)"
          :active="index === activeIndex"
          :data-command-active="kind === 'command' ? index === activeIndex : undefined"
          :data-history-active="kind === 'history' ? index === activeIndex : undefined"
          role="option"
          :aria-selected="index === activeIndex"
          tabindex="-1"
          size="sm"
          class="text-start"
          @mousedown.prevent
          @mouseenter="emit('update:activeIndex', index)"
          @click="emit('select', item)"
        >
          <slot name="item" :item="item" />
        </ListRowButton>
      </div>
    </div>
    <div class="border-t border-border/60 px-3 py-1.5 text-[11px] text-muted-foreground">{{ hint }}</div>
  </div>
</template>
