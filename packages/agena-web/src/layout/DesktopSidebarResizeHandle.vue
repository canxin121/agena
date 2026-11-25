<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { useDesktopSidebarResize, type DesktopSidebarResizeOptions } from '@/composables/useDesktopSidebarResize'
import { DESKTOP_SIDEBAR_MAX_WIDTH, DESKTOP_SIDEBAR_MIN_WIDTH, useUiStore } from '@/stores/ui'

defineProps<{ options?: DesktopSidebarResizeOptions }>()
const { t } = useI18n()
const ui = useUiStore()
const { startDesktopSidebarResize, resizeDesktopSidebarFromKeyboard } = useDesktopSidebarResize()
</script>

<template>
  <div
    role="separator"
    aria-orientation="vertical"
    :aria-label="t('header.windowTabs.resizingSidebar')"
    :aria-valuenow="Math.round(ui.sidebarWidth)"
    :aria-valuemin="DESKTOP_SIDEBAR_MIN_WIDTH"
    :aria-valuemax="DESKTOP_SIDEBAR_MAX_WIDTH"
    tabindex="0"
    class="group absolute inset-y-0 end-0 z-10 w-2 touch-none cursor-col-resize focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
    @pointerdown="startDesktopSidebarResize($event, options)"
    @keydown="resizeDesktopSidebarFromKeyboard"
  >
    <span
      class="pointer-events-none absolute inset-y-0 end-0 w-px bg-primary/50 opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100"
      aria-hidden="true"
    />
  </div>
</template>
