<script setup lang="ts">
import HelpDialog from '@/components/HelpDialog.vue'
import ImageViewerDialog from '@/components/ImageViewerDialog.vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import AppDesktopSidebar from '@/layout/AppDesktopSidebar.vue'
import AppHeader from '@/layout/AppHeader.vue'
import ChatSidebar from '@/layout/ChatSidebar.vue'
import BottomNav from '@/layout/BottomNav.vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'

import { useUiStore } from '@/stores/ui'
import { useAppRuntime } from '@/app/runtime/useAppRuntime'
import { useDesktopSidebarResize } from '@/composables/useDesktopSidebarResize'

const ui = useUiStore()
const route = useRoute()
const { t } = useI18n()
const { startDesktopSidebarResize } = useDesktopSidebarResize()
useAppRuntime()

const usesChatShellSidebar = computed(() => {
  const shellSidebar = String(route.meta?.shellSidebar || '')
    .trim()
    .toLowerCase()
  if (shellSidebar === 'chat') return true
  if (shellSidebar === 'none') return false
  return String(route.path || '')
    .toLowerCase()
    .startsWith('/chat')
})

const mobileSidebarPointerReady = ref(false)
let mobileSidebarPointerRafA: number | null = null
let mobileSidebarPointerRafB: number | null = null

function clearMobileSidebarPointerRafs() {
  if (mobileSidebarPointerRafA !== null) {
    window.cancelAnimationFrame(mobileSidebarPointerRafA)
    mobileSidebarPointerRafA = null
  }
  if (mobileSidebarPointerRafB !== null) {
    window.cancelAnimationFrame(mobileSidebarPointerRafB)
    mobileSidebarPointerRafB = null
  }
}

watch(
  () => ({
    isCompactLayout: ui.isCompactLayout,
    switcherOpen: ui.isSessionSwitcherOpen,
    usesChatShellSidebar: usesChatShellSidebar.value,
  }),
  ({ isCompactLayout, switcherOpen, usesChatShellSidebar: usesSidebar }) => {
    clearMobileSidebarPointerRafs()
    if (!isCompactLayout || !usesSidebar || !switcherOpen) {
      mobileSidebarPointerReady.value = false
      return
    }

    mobileSidebarPointerReady.value = false
    mobileSidebarPointerRafA = window.requestAnimationFrame(() => {
      mobileSidebarPointerRafA = null
      mobileSidebarPointerRafB = window.requestAnimationFrame(() => {
        mobileSidebarPointerRafB = null
        mobileSidebarPointerReady.value = true
      })
    })
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  clearMobileSidebarPointerRafs()
})

// Mobile UX: navigating should switch focus to main content.
watch(
  () => route.fullPath,
  (next, prev) => {
    if (!ui.isCompactLayout) return
    if (!ui.isSessionSwitcherOpen) return
    if (next === prev) return
    ui.setSessionSwitcherOpen(false)
  },
)

const mobileBottomNavInset =
  'calc(var(--oc-bottom-nav-height, 56px) + var(--oc-safe-area-bottom, 0px) - clamp(0px, var(--oc-keyboard-inset, 0px), var(--oc-bottom-nav-height, 56px)))'

const showBottomNav = computed(() => ui.isCompactLayout && ui.isMobileDevice)
const compactBottomInset = computed(() => (showBottomNav.value ? mobileBottomNavInset : '0px'))
const showDesktopSidebarHost = computed(() => !ui.isCompactLayout)
const isLeftSidebarResizing = ref(false)
const desktopSidebarAsideEl = ref<HTMLElement | null>(null)
const DESKTOP_SIDEBAR_COLLAPSED_WIDTH = 76

const desktopSidebarRenderWidth = computed(() => {
  if (!showDesktopSidebarHost.value) return 0
  if (ui.isSidebarOpen) return ui.sidebarWidth
  return DESKTOP_SIDEBAR_COLLAPSED_WIDTH
})

function setDesktopSidebarPreviewWidth(width: number) {
  const el = desktopSidebarAsideEl.value
  if (!el) return
  if (!(width >= 0)) return
  el.style.width = `${Math.round(width)}px`
}

function handleDesktopSidebarResize(event: PointerEvent) {
  startDesktopSidebarResize(event, {
    deferCommit: true,
    onStart: () => {
      isLeftSidebarResizing.value = true
      setDesktopSidebarPreviewWidth(ui.sidebarWidth)
    },
    onPreviewWidth: (nextWidth) => {
      setDesktopSidebarPreviewWidth(nextWidth)
    },
    onEnd: (finalWidth) => {
      setDesktopSidebarPreviewWidth(finalWidth)
      window.requestAnimationFrame(() => {
        isLeftSidebarResizing.value = false
      })
    },
  })
}

watch(
  () => showDesktopSidebarHost.value,
  (visible) => {
    if (!visible) isLeftSidebarResizing.value = false
  },
)
</script>

<template>
  <div class="main-content-safe-area relative h-[100dvh] bg-background text-foreground overflow-hidden flex flex-col">
    <HelpDialog />
    <ImageViewerDialog />

    <div class="flex flex-1 flex-col overflow-hidden">
      <AppHeader />

      <div class="flex flex-1 overflow-hidden">
        <aside
          v-if="showDesktopSidebarHost"
          ref="desktopSidebarAsideEl"
          class="relative h-full overflow-hidden bg-sidebar"
          :style="{ width: `${desktopSidebarRenderWidth}px` }"
          :class="[
            'border-r border-border',
            isLeftSidebarResizing ? '' : 'transition-[width,border-color] duration-200 ease-out',
          ]"
        >
          <div
            v-if="ui.isSidebarOpen"
            class="absolute right-0 top-0 z-30 h-full w-1 cursor-col-resize hover:bg-primary/40"
            @pointerdown="handleDesktopSidebarResize"
          />

          <div class="relative h-full min-h-0">
            <AppDesktopSidebar :expanded="ui.isSidebarOpen" :resizing="isLeftSidebarResizing" />

            <div
              v-if="ui.isSidebarOpen && isLeftSidebarResizing"
              class="oc-sidebar-resize-overlay pointer-events-none absolute inset-0 z-20 flex items-center justify-center px-3"
            >
              <div class="flex flex-col items-center gap-2 text-center">
                <Skeleton class="h-7 w-7 rounded-full" />
                <span class="text-xs font-medium text-muted-foreground">
                  {{ t('header.windowTabs.resizingSidebar') }}
                </span>
              </div>
            </div>
          </div>
        </aside>

        <div class="relative flex h-full min-w-0 flex-1 flex-col overflow-hidden">
          <div
            v-if="ui.isCompactLayout && usesChatShellSidebar"
            v-show="ui.isSessionSwitcherOpen"
            class="absolute inset-x-0 top-0 z-40 bg-sidebar"
            :class="mobileSidebarPointerReady ? '' : 'pointer-events-none'"
            :style="{ bottom: compactBottomInset }"
            :aria-hidden="!ui.isSessionSwitcherOpen"
          >
            <ChatSidebar mobile-variant />
          </div>

          <main
            class="relative min-h-0 flex-1 overflow-hidden"
            :style="ui.isCompactLayout ? { paddingBottom: compactBottomInset } : undefined"
          >
            <router-view />
          </main>
        </div>
      </div>

      <BottomNav v-if="showBottomNav" />
    </div>
  </div>
</template>
