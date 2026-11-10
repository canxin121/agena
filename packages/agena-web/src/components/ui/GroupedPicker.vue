<script setup lang="ts">
/**
 * A grouped, paginated picker built on the shared `OptionMenu`.
 *
 * `OptionMenu` already owns collapsible groups, external paging, keyboard
 * navigation, search and the desktop/mobile presentation split. This wrapper
 * only adds the trigger and turns group/item selection into one `select` event,
 * so a caller that browses a large hierarchy (sessions by project, projects by
 * page) does not re-implement any of that.
 */
import { computed, ref, type Component } from 'vue'

import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import { cn } from '@/lib/utils'

const props = withDefaults(
  defineProps<{
    /** Rendered value on the trigger; callers own the human label. */
    label: string
    placeholder?: string
    groups: OptionMenuGroup[]
    title?: string
    searchable?: boolean
    searchPlaceholder?: string
    emptyText?: string
    helperText?: string
    page?: number
    pageCount?: number
    pageLoading?: boolean
    collapsibleGroups?: boolean
    icon?: Component
    disabled?: boolean
    size?: 'default' | 'sm'
    triggerClass?: string
    menuClass?: string
    /** Fill the trigger and align groups as a list rather than a compact menu. */
    block?: boolean
    /** Keep the chosen row highlighted even after the menu closes. */
    selectedId?: string
  }>(),
  {
    placeholder: '',
    title: '',
    searchable: true,
    searchPlaceholder: '',
    emptyText: '',
    helperText: '',
    page: 1,
    pageCount: 1,
    pageLoading: false,
    collapsibleGroups: true,
    disabled: false,
    size: 'default',
    triggerClass: '',
    menuClass: '',
    block: false,
    selectedId: '',
  },
)

const emit = defineEmits<{
  (e: 'select', id: string): void
  (e: 'request-page', page: number): void
}>()

const open = ref(false)
const query = ref('')

const resolvedGroups = computed<OptionMenuGroup[]>(() =>
  props.groups.map((group) => ({
    ...group,
    items: group.items.map((item) => ({
      ...item,
      checked: Boolean(item.checked || (props.selectedId && item.id === props.selectedId)),
    })),
  })),
)

const triggerClasses = computed(() => {
  const base =
    'inline-flex min-w-0 items-center justify-between gap-2 rounded-md border border-input bg-transparent text-left shadow-sm transition-colors hover:bg-secondary/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50'
  const size = props.size === 'sm' ? 'h-8 px-2 text-xs' : 'h-9 px-3 text-sm'
  return cn(base, size, props.block ? 'w-full' : 'max-w-full', props.triggerClass)
})

function select(item: OptionMenuItem) {
  emit('select', item.id)
  // Selecting a leaf closes the picker; a group header has no id to report.
  open.value = false
}
</script>

<template>
  <div class="relative min-w-0" :class="block ? 'w-full' : ''">
    <button
      type="button"
      :disabled="disabled"
      :class="triggerClasses"
      :aria-label="title || placeholder || label"
      aria-haspopup="dialog"
      :aria-expanded="open"
      @click.stop="open = !open"
      @keydown.down.prevent="open = true"
      @keydown.up.prevent="open = true"
    >
      <span class="flex min-w-0 items-center gap-2">
        <component v-if="icon" :is="icon" class="h-4 w-4 shrink-0 text-muted-foreground" />
        <span class="min-w-0 truncate font-medium" :class="label ? '' : 'text-muted-foreground'">
          {{ label || placeholder }}
        </span>
      </span>
      <slot name="trailing" />
    </button>

    <OptionMenu
      :open="open"
      :query="query"
      :groups="resolvedGroups"
      :title="title"
      :searchable="searchable"
      :search-placeholder="searchPlaceholder"
      :empty-text="emptyText"
      :helper-text="helperText"
      :collapsible-groups="collapsibleGroups"
      :paginated="false"
      :external-page="page"
      :external-page-count="pageCount"
      :external-pager-loading="pageLoading"
      :desktop-fixed="true"
      desktop-placement="bottom-start"
      :desktop-class="cn('w-[min(26rem,calc(100vw-2rem))]', menuClass)"
      filter-mode="external"
      @update:open="open = $event"
      @update:query="query = $event"
      @select="select"
      @request-page="(page: number) => emit('request-page', page)"
    />
  </div>
</template>
