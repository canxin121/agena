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
import PickerTrigger from '@/components/ui/PickerTrigger.vue'
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
const trigger = ref<InstanceType<typeof PickerTrigger> | null>(null)
const open = ref(false)
const openAt = ref<'first' | 'last'>()
const localQuery = ref('')
const query = computed(() => props.query ?? localQuery.value)
const isCompactTouch = computed(() => props.isCompactTouch ?? ui.isCompactTouch)
const resolvedGroups = computed<OptionMenuGroup[]>(() =>
  props.groups.map((group) => ({
    ...group,
    items: group.items.map((item) => ({
      ...item,
      checked: Boolean(item.checked || (props.selectedId && item.id === props.selectedId)),
    })),
  })),
)

function select(item: OptionMenuItem) {
  if (props.disabled || item.disabled) return
  emit('select', item.id)
  // Selecting a leaf closes the picker; a group header has no id to report.
  setOpen(false)
}
function setOpen(value: boolean) {
  if (value === open.value || (value && props.disabled)) return
  open.value = value
  if (!value) openAt.value = undefined
  emit('update:open', value)
}
function openFrom(edge: 'first' | 'last') {
  openAt.value = edge
  setOpen(true)
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
    <PickerTrigger
      ref="trigger"
      :open="open"
      :block="block"
      :disabled="disabled"
      :size="size"
      :class="triggerClass"
      :label="label || placeholder"
      :aria-label="title || placeholder || label"
      :icon="icon"
      :muted="!label"
      @toggle="setOpen(!open)"
      @open-at="openFrom"
    >
      <slot name="trailing" />
    </PickerTrigger>

    <OptionMenu
      :open="open"
      :open-at="openAt"
      :query="query"
      :groups="resolvedGroups"
      :title="title"
      :searchable="searchable"
      :search-placeholder="searchPlaceholder"
      :empty-text="emptyText"
      :helper-text="helperText"
      :is-compact-touch="isCompactTouch"
      :collapsible-groups="collapsibleGroups"
      :external-page="page"
      :external-page-count="pageCount"
      :external-pager-loading="pageLoading"
      :loading="loading"
      :refreshable="refreshable"
      :refreshing="refreshing"
      :refresh-label="refreshLabel"
      :desktop-fixed="true"
      :desktop-anchor-el="trigger?.triggerEl"
      desktop-placement="bottom-start"
      :desktop-class="cn('w-[min(26rem,calc(100vw-2rem))]', menuClass)"
      :filter-mode="filterMode"
      @update:open="setOpen"
      @update:query="setQuery"
      @select="select"
      @request-page="(page: number) => emit('request-page', page)"
      @refresh="emit('refresh')"
    />
  </div>
</template>
