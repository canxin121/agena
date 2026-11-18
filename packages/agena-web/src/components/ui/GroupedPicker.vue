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
import { computed, ref, watch, type Component } from 'vue'

import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import { cn } from '@/lib/utils'
import { useUiStore } from '@/stores/ui'

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
    /** Controlled search; external callers load their own filtered pages. */
    query?: string
    filterMode?: 'internal' | 'external'
    loading?: boolean
    refreshable?: boolean
    refreshing?: boolean
    refreshLabel?: string
    isCompactTouch?: boolean
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
    filterMode: 'internal',
    loading: false,
    refreshable: false,
    refreshing: false,
    refreshLabel: '',
    isCompactTouch: undefined,
  },
)

const emit = defineEmits<{
  (e: 'select', id: string): void
  (e: 'request-page', page: number): void
  (e: 'update:query', query: string): void
  (e: 'update:open', open: boolean): void
  (e: 'refresh'): void
}>()

const ui = useUiStore()
const open = ref(false)
const localQuery = ref('')
const query = computed(() => props.query ?? localQuery.value)
const isCompactTouch = computed(() => props.isCompactTouch ?? ui.isCompactTouch)
const filteredGroups = computed(() => {
  const search = query.value.trim().toLowerCase()
  if (props.filterMode === 'external' || !props.searchable || !search) return props.groups
  return props.groups
    .map((group) => ({
      ...group,
      items: group.items.filter((item) =>
        [item.id, item.label, item.description, item.keywords].join(' ').toLowerCase().includes(search),
      ),
    }))
    .filter((group) => group.items.length > 0)
})

const resolvedGroups = computed<OptionMenuGroup[]>(() =>
  filteredGroups.value.map((group) => ({
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
  if (props.disabled || item.disabled) return
  emit('select', item.id)
  // Selecting a leaf closes the picker; a group header has no id to report.
  setOpen(false)
}
function setOpen(value: boolean) {
  if (value && props.disabled) return
  open.value = value
  emit('update:open', value)
}
function setQuery(value: string) {
  localQuery.value = value
  emit('update:query', value)
}
watch(
  () => props.disabled,
  (disabled) => {
    if (disabled && open.value) setOpen(false)
  },
)
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
      @click.stop="setOpen(!open)"
      @keydown.down.prevent="setOpen(true)"
      @keydown.up.prevent="setOpen(true)"
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
      :is-compact-touch="isCompactTouch"
      :collapsible-groups="collapsibleGroups"
      :paginated="false"
      :external-page="page"
      :external-page-count="pageCount"
      :external-pager-loading="pageLoading"
      :loading="loading"
      :refreshable="refreshable"
      :refreshing="refreshing"
      :refresh-label="refreshLabel"
      :desktop-fixed="true"
      desktop-placement="bottom-start"
      :desktop-class="cn('w-[min(26rem,calc(100vw-2rem))]', menuClass)"
      filter-mode="external"
      @update:open="setOpen"
      @update:query="setQuery"
      @select="select"
      @request-page="(page: number) => emit('request-page', page)"
      @refresh="emit('refresh')"
    />
  </div>
</template>
