<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCloseLine, RiListCheck3, RiRefreshLine } from '@remixicon/vue'
import SidebarIconButton from '@/components/ui/SidebarIconButton.vue'
import {
  enabledKeyboardItems,
  focusKeyboardItem,
  handleTabKeydown,
  isVisibleKeyboardTarget,
} from '@/lib/keyboardNavigation'

const props = defineProps<{
  selectableIds: readonly string[]
  selectedIds: ReadonlySet<string>
  selectionLabel: string
  countText?: string
  exitLabel: string
  compact?: boolean
  isTouchPointer?: boolean
}>()
const emit = defineEmits<{ selectAll: []; invert: []; exit: [] }>()
const { t } = useI18n()
const toolbar = ref<HTMLElement | null>(null)
const buttonSize = computed(() => (props.compact && !props.isTouchPointer ? 'sm' : 'md'))
const canSelectAll = computed(() => props.selectableIds.some((id) => !props.selectedIds.has(id)))

onMounted(() => {
  if (toolbar.value?.closest('[data-selection-context]') && document.activeElement === document.body) {
    focusKeyboardItem(enabledKeyboardItems(toolbar.value)[0])
  }
})

async function selectAll(event: MouseEvent) {
  const trigger = event.currentTarget
  const restoreFocus = document.activeElement === trigger
  emit('selectAll')
  if (!restoreFocus) return
  await nextTick()
  if (!canSelectAll.value && (document.activeElement === document.body || document.activeElement === trigger)) {
    focusKeyboardItem(enabledKeyboardItems(toolbar.value)[0])
  }
}

async function exit() {
  const context = toolbar.value?.closest('[data-selection-context]')
  const restoreFocus = toolbar.value?.contains(document.activeElement)
  emit('exit')
  if (!context || !restoreFocus) return
  await nextTick()
  const trigger = context.querySelector<HTMLElement>('[data-selection-trigger]')
  if (document.activeElement === document.body && trigger && isVisibleKeyboardTarget(trigger)) {
    trigger.focus({ preventScroll: true })
  }
}
</script>

<template>
  <span
    ref="toolbar"
    data-selection-toolbar
    role="group"
    :aria-label="t('common.actions')"
    class="flex min-w-0 items-center justify-between gap-1"
    :class="compact ? 'flex-nowrap' : 'min-h-8 flex-wrap px-2 py-1'"
    @keydown="handleTabKeydown($event, false)"
  >
    <span
      class="max-w-full truncate rounded-full bg-muted px-1.5 text-[10px] leading-4 font-medium text-foreground/80 tabular-nums"
      :title="selectionLabel"
      role="status"
    >
      <template v-if="countText">
        <span aria-hidden="true">{{ countText }}</span>
        <span class="sr-only">{{ selectionLabel }}</span>
      </template>
      <template v-else>{{ selectionLabel }}</template>
    </span>

    <span class="flex items-center gap-0.5">
      <SidebarIconButton
        data-keyboard-option
        :size="buttonSize"
        :tooltip="t('common.selectAll')"
        :is-touch-pointer="isTouchPointer"
        :disabled="!canSelectAll"
        @click.stop="selectAll"
      >
        <RiListCheck3 class="size-3.5" aria-hidden="true" />
      </SidebarIconButton>
      <SidebarIconButton
        data-keyboard-option
        :size="buttonSize"
        :tooltip="t('common.invertSelection')"
        :is-touch-pointer="isTouchPointer"
        :disabled="!selectableIds.length"
        @click.stop="emit('invert')"
      >
        <RiRefreshLine class="size-3.5" aria-hidden="true" />
      </SidebarIconButton>

      <slot :size="buttonSize" />

      <SidebarIconButton
        data-keyboard-option
        :size="buttonSize"
        :tooltip="exitLabel"
        :is-touch-pointer="isTouchPointer"
        @click.stop="exit"
      >
        <RiCloseLine class="size-3.5" aria-hidden="true" />
      </SidebarIconButton>
    </span>
  </span>
</template>
