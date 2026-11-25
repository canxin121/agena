<script setup lang="ts">
import { useMobileSheetLayout } from '@/composables/useMobileSheetLayout'
import { createReusableTemplate } from '@vueuse/core'
import { computed, nextTick, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowDownSLine, RiCloseLine, RiRefreshLine } from '@remixicon/vue'
import { DialogTitle } from 'radix-vue'

import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import ModalSurface from '@/components/ui/ModalSurface.vue'
import OptionMenuRow from '@/components/ui/OptionMenuRow.vue'
import PaginationControls from '@/components/ui/PaginationControls.vue'
import PopoverSurface from '@/components/ui/PopoverSurface.vue'
import { lockListItemActionsForAnchor } from '@/components/ui/listItemActionVisibilityLock'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import { normalizeOptionSearch, optionSearchText } from '@/components/ui/optionSearch'
import { cn } from '@/lib/utils'
import { resolveOverlayAnchor, resolveOverlayAnchorElement, type OverlayAnchor } from '@/lib/overlayAnchor'
import {
  enabledKeyboardItems,
  focusKeyboardItem,
  handleOptionKeydown,
  isPlainNavigationKey,
  isVisibleKeyboardTarget,
} from '@/lib/keyboardNavigation'
import { rememberKeyboardOverlaySource, useDialogReturnFocus } from '@/composables/useDialogReturnFocus'

type ResolvedGroup = OptionMenuGroup & {
  _key: string
}

const props = withDefaults(
  defineProps<{
    open: boolean
    query?: string
    groups: OptionMenuGroup[]
    title?: string
    mobileTitle?: string
    searchable?: boolean
    searchPlaceholder?: string
    emptyText?: string
    helperText?: string
    isCompactTouch?: boolean
    closeLabel?: string
    desktopPlacement?: 'top-start' | 'top-end' | 'bottom-start' | 'bottom-end'
    desktopClass?: string
    desktopStyle?: CSSProperties
    desktopFixed?: boolean
    desktopAnchorEl?: OverlayAnchor
    desktopContentMaxHeightClass?: string
    desktopGapPx?: number
    desktopViewportMarginPx?: number
    filterMode?: 'internal' | 'external'
    closeOnSelect?: boolean
    autoFocusSearch?: boolean
    openAt?: 'first' | 'last'
    paginated?: boolean
    pageSize?: number
    paginationMode?: 'group' | 'item'
    collapsibleGroups?: boolean
    externalPage?: number
    externalPageCount?: number
    externalPagerLoading?: boolean
    loading?: boolean
    loadingSkeletonRows?: number
    refreshable?: boolean
    refreshing?: boolean
    refreshLabel?: string
    onRefresh?: (() => void | Promise<void>) | null
    /** Layout actions may replace the trigger with a surviving destination. */
    returnFocusTarget?: () => HTMLElement | null
  }>(),
  {
    query: '',
    title: '',
    mobileTitle: '',
    searchable: true,
    searchPlaceholder: '',
    emptyText: '',
    helperText: '',
    isCompactTouch: false,
    closeLabel: '',
    desktopPlacement: 'bottom-start',
    desktopClass: 'w-64',
    desktopStyle: undefined,
    desktopFixed: true,
    desktopAnchorEl: null,
    desktopContentMaxHeightClass: 'max-h-64',
    desktopGapPx: 8,
    desktopViewportMarginPx: 8,
    filterMode: 'internal',
    closeOnSelect: true,
    autoFocusSearch: true,
    paginated: false,
    pageSize: 80,
    paginationMode: 'group',
    collapsibleGroups: false,
    externalPage: 1,
    externalPageCount: 1,
    externalPagerLoading: false,
    loading: false,
    loadingSkeletonRows: 6,
    refreshable: false,
    refreshing: false,
    refreshLabel: '',
    onRefresh: null,
  },
)

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'update:query', value: string): void
  (e: 'select', item: OptionMenuItem): void
  (e: 'request-page', page: number): void
  (e: 'refresh'): void
  (e: 'close'): void
}>()

const { t } = useI18n()
const [DefineMenuBody, MenuBody] = createReusableTemplate()
const restoreMissingTrigger = useDialogReturnFocus(() => props.open)

const effectiveSearchPlaceholder = computed(() =>
  props.searchPlaceholder && props.searchPlaceholder.trim() ? props.searchPlaceholder : t('common.searchOptions'),
)
const effectiveEmptyText = computed(() =>
  props.emptyText && props.emptyText.trim() ? props.emptyText : t('common.noOptionsFound'),
)
const effectiveCloseLabel = computed(() =>
  props.closeLabel && props.closeLabel.trim() ? props.closeLabel : t('common.close'),
)
const effectiveRefreshLabel = computed(() =>
  props.refreshLabel && props.refreshLabel.trim() ? props.refreshLabel : t('common.refresh'),
)

const rootEl = ref<HTMLElement | null>(null)
const panelEl = ref<HTMLElement | null>(null)
let unmounting = false
const collapsedGroupByKey = ref<Record<string, boolean>>({})
const currentPage = ref(1)
const localRefreshing = ref(false)

const isMobileSheet = computed(() => Boolean(props.isCompactTouch))
const normalizedQuery = computed(() => normalizeOptionSearch(props.query))
const effectivePageSize = computed(() => Math.max(1, Math.floor(Number(props.pageSize) || 80)))

function toNonNegativePx(value: unknown, fallback: number): number {
  const n = Number(value)
  if (!Number.isFinite(n)) return fallback
  return Math.max(0, Math.round(n))
}

const panelGapPx = computed(() => toNonNegativePx(props.desktopGapPx, 8))
const panelViewportMarginPx = computed(() => toNonNegativePx(props.desktopViewportMarginPx, 8))
const desktopPopoverSide = computed<'top' | 'bottom'>(() =>
  props.desktopPlacement.startsWith('top') ? 'top' : 'bottom',
)
const desktopPopoverAlign = computed<'start' | 'end'>(() => (props.desktopPlacement.endsWith('end') ? 'end' : 'start'))

const mobileSheetStyle = useMobileSheetLayout(() => props.open && isMobileSheet.value)

let releaseActionLock: (() => void) | null = null
let returnFocusEl: HTMLElement | null = null
let lastPanelFocus: HTMLElement | null = null

const anchorElement = computed(
  () => resolveOverlayAnchorElement(props.desktopAnchorEl) || rootEl.value?.parentElement || rootEl.value,
)
const desktopPopoverAnchor = computed(() => resolveOverlayAnchor(props.desktopAnchorEl) ?? anchorElement.value)

function clearActionLock() {
  if (!releaseActionLock) return
  releaseActionLock()
  releaseActionLock = null
}

function syncActionLock() {
  clearActionLock()
  if (!props.open || isMobileSheet.value) return
  releaseActionLock = lockListItemActionsForAnchor(anchorElement.value)
}

function itemHaystack(item: OptionMenuItem): string {
  return optionSearchText(item.id, item.label, item.description, item.keywords)
}

const allGroups = computed<ResolvedGroup[]>(() => {
  const groups = Array.isArray(props.groups) ? props.groups : []
  return groups.map((group, index) => ({
    ...group,
    items: Array.isArray(group.items) ? group.items : [],
    _key: String(group.id || `group-${index}`),
  }))
})

const visibleGroups = computed<ResolvedGroup[]>(() => {
  const groups = allGroups.value
  if (props.filterMode === 'external' || !props.searchable || !normalizedQuery.value) return groups

  const q = normalizedQuery.value
  return groups
    .map((group) => ({
      ...group,
      items: (group.items || []).filter((item) => itemHaystack(item).includes(q)),
    }))
    .filter((group) => (group.items || []).length > 0)
})

watch(
  () => visibleGroups.value,
  (groups) => {
    const next = { ...collapsedGroupByKey.value }
    for (const group of groups) {
      if (!(group._key in next)) {
        next[group._key] = Boolean(group.defaultCollapsed)
      }
    }
    collapsedGroupByKey.value = next
  },
  { immediate: true },
)

function canCollapseGroup(group: ResolvedGroup): boolean {
  if (!props.collapsibleGroups) return false
  if (group.collapsible === false) return false
  return Boolean(group.title || group.subtitle)
}

function isGroupCollapsed(group: ResolvedGroup): boolean {
  if (!canCollapseGroup(group)) return false
  return Boolean(collapsedGroupByKey.value[group._key])
}

function toggleGroupCollapsed(group: ResolvedGroup) {
  if (!canCollapseGroup(group)) return
  collapsedGroupByKey.value = {
    ...collapsedGroupByKey.value,
    [group._key]: !isGroupCollapsed(group),
  }
}

const groupedPages = computed<ResolvedGroup[][]>(() => {
  const groups = visibleGroups.value
  if (!props.paginated) return [groups]
  if (groups.length === 0) return [[]]

  const pageSize = effectivePageSize.value

  if (props.paginationMode === 'item') {
    const pages: ResolvedGroup[][] = []
    let currentGroups: ResolvedGroup[] = []
    let currentGroupMap = new Map<string, ResolvedGroup>()
    let currentCount = 0

    for (const group of groups) {
      for (const item of group.items) {
        if (currentCount >= pageSize) {
          pages.push(currentGroups)
          currentGroups = []
          currentGroupMap = new Map<string, ResolvedGroup>()
          currentCount = 0
        }

        let bucket = currentGroupMap.get(group._key)
        if (!bucket) {
          bucket = {
            ...group,
            items: [],
          }
          currentGroupMap.set(group._key, bucket)
          currentGroups.push(bucket)
        }
        bucket.items.push(item)
        currentCount += 1
      }
    }

    if (currentGroups.length > 0) pages.push(currentGroups)
    return pages.length > 0 ? pages : [[]]
  }

  const pages: ResolvedGroup[][] = []
  let currentGroups: ResolvedGroup[] = []
  let currentCount = 0

  for (const group of groups) {
    const groupCount = group.items.length

    if (currentGroups.length === 0) {
      currentGroups = [group]
      currentCount = groupCount
      continue
    }

    if (currentCount + groupCount > pageSize) {
      pages.push(currentGroups)
      currentGroups = [group]
      currentCount = groupCount
      continue
    }

    currentGroups.push(group)
    currentCount += groupCount
  }

  if (currentGroups.length > 0) pages.push(currentGroups)
  return pages.length > 0 ? pages : [[]]
})

const pageCount = computed(() => Math.max(1, groupedPages.value.length))

const pagedGroups = computed<ResolvedGroup[]>(() => {
  const idx = Math.max(0, Math.min(pageCount.value - 1, currentPage.value - 1))
  return groupedPages.value[idx] || []
})

const totalVisibleItemCount = computed(() => visibleGroups.value.reduce((sum, group) => sum + group.items.length, 0))
const hasVisibleItems = computed(() => totalVisibleItemCount.value > 0)
const showRefreshButton = computed(() => props.refreshable || typeof props.onRefresh === 'function')
const refreshBusy = computed(() => props.refreshing || localRefreshing.value)
const refreshDisabled = computed(() => refreshBusy.value || props.loading)
const loadingSkeletonRows = computed(() => {
  const count = Math.max(3, Math.floor(Number(props.loadingSkeletonRows) || 6))
  return Array.from({ length: count }, (_, idx) => idx)
})

const showPager = computed(() => props.paginated && hasVisibleItems.value && pageCount.value > 1)

const externalPagerTotalPages = computed(() => Math.max(1, Math.floor(Number(props.externalPageCount) || 1)))
const externalPagerCurrentPage = computed(() => {
  const page = Math.floor(Number(props.externalPage) || 1)
  return Math.max(1, Math.min(externalPagerTotalPages.value, page))
})
const useExternalPager = computed(() => externalPagerTotalPages.value > 1)
const effectiveShowPager = computed(() =>
  useExternalPager.value ? externalPagerTotalPages.value > 1 : showPager.value,
)
const pagerCurrentPage = computed(() => (useExternalPager.value ? externalPagerCurrentPage.value : currentPage.value))
const pagerTotalPages = computed(() => (useExternalPager.value ? externalPagerTotalPages.value : pageCount.value))
const pagerBusy = computed(() => useExternalPager.value && props.externalPagerLoading)

watch(
  () => pageCount.value,
  (count) => {
    if (currentPage.value > count) currentPage.value = count
    if (currentPage.value < 1) currentPage.value = 1
  },
  { immediate: true },
)

watch(
  () => normalizedQuery.value,
  () => {
    if (!props.paginated) return
    currentPage.value = 1
  },
)

function closeMenu() {
  emit('update:open', false)
  emit('close')
}

function onRadixSurfaceOpenChange(next: boolean) {
  if (!next && props.open) closeMenu()
}

function captureReturnFocus() {
  if (typeof document === 'undefined') return
  const active = document.activeElement
  const anchor = resolveOverlayAnchorElement(props.desktopAnchorEl)
  returnFocusEl =
    active instanceof HTMLElement && active !== document.body
      ? active
      : anchor?.matches('button, a[href], [tabindex]')
        ? anchor
        : anchor?.querySelector<HTMLElement>('button, a[href], [tabindex]') || null
}

watch(
  [() => props.open, panelEl],
  ([open, panel]) => {
    if (open && panel && props.closeOnSelect && returnFocusEl) rememberKeyboardOverlaySource(panel, returnFocusEl)
  },
  { flush: 'post' },
)

async function restoreReturnFocus() {
  if (props.open && !unmounting) return
  const source = returnFocusEl
  returnFocusEl = null
  if (typeof document === 'undefined') return
  await nextTick()
  if (props.open && !unmounting) return
  const target = props.returnFocusTarget?.() || source
  if (!target || !isVisibleKeyboardTarget(target) || target.matches(':disabled')) return

  const active = document.activeElement
  if (
    active instanceof HTMLElement &&
    active !== document.body &&
    active !== document.documentElement &&
    !(props.returnFocusTarget && active === source)
  )
    return
  target.focus({ preventScroll: true })
}

function onCloseAutoFocus(event: Event) {
  // PopoverAnchor does not register a Radix trigger. Without this, Radix's
  // unmount callback can move focus back to body after our own restoration.
  event.preventDefault()
  if (!props.returnFocusTarget) restoreMissingTrigger(event)
  void restoreReturnFocus()
}

function onInteractOutside(event: CustomEvent<{ originalEvent: Event }>) {
  const target = event.detail.originalEvent.target
  const anchor = resolveOverlayAnchorElement(props.desktopAnchorEl)
  if (target instanceof Node && anchor?.contains(target)) event.preventDefault()
}

function selectItem(item: OptionMenuItem) {
  if (item.disabled) return
  emit('select', item)
  if (props.closeOnSelect) closeMenu()
}

function updateQuery(value: string | number) {
  emit('update:query', String(value || ''))
}

function setPagerPage(nextPage: number) {
  if (pagerBusy.value) return
  const target = Math.max(1, Math.min(pagerTotalPages.value, Math.floor(nextPage || 1)))
  if (target === pagerCurrentPage.value) return
  if (useExternalPager.value) {
    emit('request-page', target)
    return
  }
  currentPage.value = target
}

async function triggerRefresh() {
  if (!showRefreshButton.value || refreshDisabled.value) return
  const callback = props.onRefresh
  if (typeof callback === 'function') {
    localRefreshing.value = true
    try {
      await callback()
    } finally {
      localRefreshing.value = false
    }
    return
  }
  emit('refresh')
}

function searchField(panel = panelEl.value): HTMLInputElement | null {
  return panel?.querySelector<HTMLInputElement>('[data-option-menu-search]') || null
}

function onPanelKeydown(event: KeyboardEvent) {
  const search = searchField()
  if (handleOptionKeydown(event, panelEl.value, search)) return
  if (!search || event.target !== search || event.key !== 'Enter' || event.shiftKey || !isPlainNavigationKey(event))
    return

  const [first] = enabledKeyboardItems(panelEl.value, '[data-option-menu-item]')
  event.preventDefault()
  event.stopPropagation()
  first?.click()
}

function onOpenAutoFocus(event: Event) {
  const panel = panelEl.value
  if (!panel || !props.open) return
  event.preventDefault()
  const search = !isMobileSheet.value && props.autoFocusSearch ? searchField(panel) : null
  if (!props.openAt && search) {
    search.focus({ preventScroll: true })
    return
  }
  const items = enabledKeyboardItems(panel)
  const target =
    props.openAt === 'last'
      ? items.at(-1)
      : props.openAt === 'first'
        ? items[0]
        : items.find((item) => item.getAttribute('aria-current') === 'true') || items[0]
  if (isMobileSheet.value && props.searchable && !props.openAt) panel.focus({ preventScroll: true })
  else if (!focusKeyboardItem(target)) (search || panel).focus({ preventScroll: true })
}

function onPanelFocusin(event: FocusEvent) {
  if (event.target instanceof HTMLElement) lastPanelFocus = event.target
}

watch(
  [
    pagedGroups,
    collapsedGroupByKey,
    () => props.loading,
    refreshDisabled,
    effectiveShowPager,
    pagerCurrentPage,
    pagerBusy,
  ],
  () => {
    if (!props.open || !lastPanelFocus || !panelEl.value) return
    const active = document.activeElement
    if (active !== document.body && active !== lastPanelFocus) return
    if (isVisibleKeyboardTarget(lastPanelFocus) && !lastPanelFocus.matches(':disabled')) return
    const [first] = enabledKeyboardItems(panelEl.value)
    if (!focusKeyboardItem(first))
      (isMobileSheet.value ? panelEl.value : searchField() || panelEl.value).focus({ preventScroll: true })
  },
  { flush: 'post' },
)

function containsTarget(target: Node | null): boolean {
  if (!target) return false
  return Boolean(rootEl.value?.contains(target) || panelEl.value?.contains(target))
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      captureReturnFocus()
      if (props.paginated) currentPage.value = 1
      return
    }
    lastPanelFocus = null
  },
  { immediate: true },
)

watch([() => props.open, isMobileSheet, anchorElement], syncActionLock, { immediate: true })

onBeforeUnmount(() => {
  unmounting = true
  clearActionLock()
})

defineExpose({ containsTarget })
</script>

<template>
  <DefineMenuBody>
    <div
      v-if="isMobileSheet || title || showRefreshButton"
      class="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-2"
    >
      <DialogTitle v-if="isMobileSheet" class="min-w-0 flex-1 truncate text-sm font-semibold">
        {{ mobileTitle || title || t('common.options') }}
      </DialogTitle>
      <div v-else class="min-w-0 flex-1 truncate text-xs font-semibold">{{ title }}</div>
      <IconButton
        v-if="showRefreshButton"
        size="sm"
        class="text-muted-foreground"
        :aria-label="effectiveRefreshLabel"
        :title="effectiveRefreshLabel"
        :disabled="refreshDisabled"
        @click="void triggerRefresh()"
      >
        <RiRefreshLine class="h-4 w-4" :class="refreshBusy ? 'animate-spin' : ''" aria-hidden="true" />
      </IconButton>
      <IconButton
        v-if="isMobileSheet"
        size="sm"
        class="size-10"
        is-touch-pointer
        :aria-label="effectiveCloseLabel"
        :title="effectiveCloseLabel"
        @click="closeMenu"
      >
        <RiCloseLine class="h-4 w-4" aria-hidden="true" />
      </IconButton>
    </div>

    <div v-if="searchable" class="shrink-0 border-b border-border/50" :class="isMobileSheet ? 'p-3' : 'p-2'">
      <Input
        data-option-menu-search
        :aria-label="effectiveSearchPlaceholder"
        :model-value="query"
        :placeholder="effectiveSearchPlaceholder"
        :class="isMobileSheet ? 'h-10 text-sm' : 'h-8 text-xs'"
        @update:model-value="updateQuery"
      />
    </div>

    <div
      class="min-h-0 flex-1 overflow-auto overscroll-contain"
      :class="isMobileSheet ? 'p-2' : ['p-1', desktopContentMaxHeightClass]"
      :aria-busy="loading"
    >
      <div v-if="loading" class="space-y-1.5 p-1" aria-hidden="true">
        <div
          v-for="index in loadingSkeletonRows"
          :key="index"
          class="flex animate-pulse gap-2 rounded-lg border border-border/35 bg-secondary/20 p-2"
          :class="isMobileSheet ? 'min-h-11 items-center' : 'min-h-9'"
        >
          <div class="h-4 w-4 shrink-0 rounded bg-muted-foreground/25" />
          <div class="min-w-0 flex-1 space-y-1">
            <div class="h-2.5 w-2/3 rounded bg-muted-foreground/25" />
            <div class="h-2 w-1/2 rounded bg-muted-foreground/25" />
          </div>
        </div>
      </div>

      <template v-else-if="hasVisibleItems">
        <div
          v-for="(group, groupIndex) in pagedGroups"
          :key="group._key"
          :class="groupIndex > 0 && 'mt-1 border-t border-border/40 pt-1'"
        >
          <div v-if="group.title || group.subtitle" class="px-2 py-1">
            <component
              :is="canCollapseGroup(group) ? 'button' : 'div'"
              :type="canCollapseGroup(group) ? 'button' : undefined"
              :data-keyboard-option="canCollapseGroup(group) ? '' : undefined"
              :aria-expanded="canCollapseGroup(group) ? !isGroupCollapsed(group) : undefined"
              class="flex w-full items-start gap-1.5 rounded px-1 py-1 text-start text-muted-foreground"
              :class="
                canCollapseGroup(group) && 'hover:bg-secondary/40 focus-visible:outline-2 focus-visible:outline-ring'
              "
              @click="canCollapseGroup(group) && toggleGroupCollapsed(group)"
            >
              <RiArrowDownSLine
                v-if="canCollapseGroup(group)"
                class="h-4 w-4 shrink-0 transition-transform"
                :class="isGroupCollapsed(group) && '-rotate-90 rtl:rotate-90'"
                aria-hidden="true"
              />
              <div class="min-w-0 flex-1">
                <div v-if="group.title" class="text-[11px] font-semibold">{{ group.title }}</div>
                <div v-if="group.subtitle" class="text-[11px]">{{ group.subtitle }}</div>
              </div>
              <span v-if="canCollapseGroup(group)" class="text-[11px] tabular-nums">{{ group.items.length }}</span>
            </component>
          </div>

          <template v-if="!isGroupCollapsed(group)">
            <template v-for="item in group.items" :key="item.id">
              <ConfirmPopover
                v-if="item.confirmTitle"
                :title="item.confirmTitle"
                :description="item.confirmDescription || ''"
                :confirm-text="item.confirmText || t('common.confirm')"
                :cancel-text="item.cancelText || t('common.cancel')"
                :variant="item.variant"
                @confirm="selectItem(item)"
              >
                <OptionMenuRow :item="item" :compact="isMobileSheet" @click.stop />
              </ConfirmPopover>
              <OptionMenuRow v-else :item="item" :compact="isMobileSheet" @click="selectItem(item)" />
            </template>
          </template>
        </div>
      </template>

      <div v-else class="px-3 py-3 text-muted-foreground" :class="isMobileSheet ? 'text-sm' : 'text-xs'">
        {{ effectiveEmptyText }}
      </div>
      <p v-if="helperText && !loading" class="px-3 py-2 text-xs text-muted-foreground">{{ helperText }}</p>
    </div>

    <div v-if="effectiveShowPager" class="shrink-0 border-t border-border/50 px-2 py-1.5">
      <PaginationControls
        class="w-full justify-center"
        :size="isMobileSheet ? 'md' : 'sm'"
        :page="pagerCurrentPage"
        :total-pages="pagerTotalPages"
        :disabled="pagerBusy"
        :prev-label="t('common.previousPage')"
        :next-label="t('common.nextPage')"
        :page-input-label="t('common.currentPage')"
        @update:page="setPagerPage"
      />
    </div>
  </DefineMenuBody>

  <div ref="rootEl" class="contents">
    <PopoverSurface
      :open="open && !isMobileSheet"
      :anchor="desktopPopoverAnchor"
      :portalled="desktopFixed"
      :layer="75"
      as-child
      :side="desktopPopoverSide"
      :align="desktopPopoverAlign"
      :side-offset="panelGapPx"
      :collision-padding="panelViewportMarginPx"
      update-position-strategy="always"
      :prioritize-position="true"
      @update:open="onRadixSurfaceOpenChange"
      @open-auto-focus="onOpenAutoFocus"
      @close-auto-focus="onCloseAutoFocus"
      @interact-outside="onInteractOutside"
    >
      <div
        ref="panelEl"
        data-oc-keyboard-tap="keep"
        :aria-label="title || t('common.options')"
        :class="cn('pointer-events-auto flex flex-col overflow-hidden', desktopClass)"
        :style="desktopStyle"
        @pointerdown.stop
        @click.stop
        @keydown="onPanelKeydown"
        @focusin="onPanelFocusin"
      >
        <MenuBody />
      </div>
    </PopoverSurface>

    <ModalSurface
      :open="open && isMobileSheet"
      :layer="74"
      as-child
      :aria-describedby="undefined"
      @update:open="onRadixSurfaceOpenChange"
      @open-auto-focus="onOpenAutoFocus"
      @close-auto-focus="onCloseAutoFocus"
    >
      <div
        ref="panelEl"
        data-oc-keyboard-tap="keep"
        data-keyboard-overlay="true"
        tabindex="-1"
        class="oc-overlay-panel fixed flex max-w-[24rem] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden"
        :style="mobileSheetStyle"
        @pointerdown.stop
        @click.stop
        @keydown="onPanelKeydown"
        @focusin="onPanelFocusin"
      >
        <MenuBody />
      </div>
    </ModalSurface>
  </div>
</template>
