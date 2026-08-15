<script setup lang="ts">
import { computed, type Component } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftSLine, RiArrowRightSLine, RiChat4Line, RiHome4Line, RiSettings3Line } from '@remixicon/vue'

import ChatSidebar from '@/layout/ChatSidebar.vue'
import { mainTabFromPath, WORKSPACE_MAIN_TABS, type MainTabId } from '@/app/navigation/mainTabs'
import { cn } from '@/lib/utils'
import { useUiStore } from '@/stores/ui'
import { useChatStore } from '@/stores/chat'

const props = withDefaults(
  defineProps<{
    expanded?: boolean
    resizing?: boolean
  }>(),
  {
    expanded: true,
    resizing: false,
  },
)

type NavItem = {
  id: MainTabId
  label: string
  to: string
  icon: Component
}

const ui = useUiStore()
const chat = useChatStore()
const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const NAV_ICONS: Record<MainTabId, Component> = {
  hub: RiHome4Line,
  chat: RiChat4Line,
  settings: RiSettings3Line,
}

function routeForTab(tabId: MainTabId): string {
  return WORKSPACE_MAIN_TABS.find((item) => item.id === tabId)?.path ?? '/chat'
}

const activeTab = computed<MainTabId>(() => mainTabFromPath(String(route.path || '')))

const navItems = computed<NavItem[]>(() =>
  WORKSPACE_MAIN_TABS.map((tab) => ({
    id: tab.id,
    label: String(t(tab.labelKey)),
    to: routeForTab(tab.id),
    icon: NAV_ICONS[tab.id],
  })),
)

const primaryRailItems = computed(() => navItems.value.filter((item) => item.id !== 'settings'))
const settingsNavItem = computed(() => navItems.value.find((item) => item.id === 'settings') || null)

function isTabActive(tabId: MainTabId): boolean {
  return activeTab.value === tabId
}

function toggleSidebarExpansion() {
  ui.toggleSidebar()
}

async function locateCurrentSessionInSidebar() {
  const sid = String(chat.selectedSessionId || '').trim()
  if (!sid) return
  ui.openAndLocateSessionInSidebar(sid)
  if (activeTab.value !== 'chat') {
    await router.push('/chat')
  }
}

async function openSettings() {
  ui.setSidebarOpen(true, { preserveWidth: true })
  await router.push('/settings')
}

function railItemAriaLabel(item: NavItem): string {
  if (item.id === 'chat' && isTabActive('chat') && String(chat.selectedSessionId || '').trim()) {
    return String(t('header.locateCurrentSession'))
  }
  return item.label
}

async function activateRailItem(tabId: MainTabId) {
  if (tabId === 'chat' && isTabActive('chat') && String(chat.selectedSessionId || '').trim()) {
    await locateCurrentSessionInSidebar()
    return
  }

  if (tabId === 'settings') {
    await openSettings()
    return
  }

  await router.push(routeForTab(tabId))
}
</script>

<template>
  <div class="flex h-full min-h-0 overflow-hidden bg-sidebar text-sidebar-foreground">
    <div class="flex h-full w-[76px] shrink-0 flex-col border-r border-sidebar-border/70 bg-sidebar/95 px-2 py-3">
      <div class="mb-3 flex justify-center">
        <button
          type="button"
          class="flex h-11 w-11 items-center justify-center rounded-2xl border border-sidebar-border/70 bg-background/60 text-foreground transition-colors hover:bg-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :title="props.expanded ? String(t('desktopSidebar.collapse')) : String(t('desktopSidebar.expand'))"
          :aria-label="props.expanded ? String(t('desktopSidebar.collapse')) : String(t('desktopSidebar.expand'))"
          @click="toggleSidebarExpansion"
        >
          <component :is="props.expanded ? RiArrowLeftSLine : RiArrowRightSLine" class="h-5 w-5" />
        </button>
      </div>

      <nav class="flex flex-1 flex-col gap-1.5" :aria-label="String(t('aria.primaryNavigation'))">
        <button
          v-for="item in primaryRailItems"
          :key="item.id"
          type="button"
          :title="railItemAriaLabel(item)"
          :aria-label="railItemAriaLabel(item)"
          :aria-current="isTabActive(item.id) ? 'page' : undefined"
          :class="
            cn(
              'group relative mx-auto flex h-11 w-11 items-center justify-center rounded-2xl border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              isTabActive(item.id)
                ? 'border-primary/35 bg-primary/12 text-primary shadow-[inset_0_1px_0_rgba(255,255,255,0.35)]'
                : 'border-transparent text-muted-foreground hover:border-sidebar-border/70 hover:bg-background/70 hover:text-foreground',
            )
          "
          @click="void activateRailItem(item.id)"
        >
          <component :is="item.icon" class="h-5 w-5" />
        </button>
      </nav>

      <div v-if="settingsNavItem" class="mt-auto flex flex-col gap-1.5 border-t border-sidebar-border/70 pt-3">
        <button
          type="button"
          :title="settingsNavItem.label"
          :aria-label="settingsNavItem.label"
          :aria-current="isTabActive('settings') ? 'page' : undefined"
          :class="
            cn(
              'group relative mx-auto flex h-11 w-11 items-center justify-center rounded-2xl border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              isTabActive('settings')
                ? 'border-primary/35 bg-primary/12 text-primary shadow-[inset_0_1px_0_rgba(255,255,255,0.35)]'
                : 'border-transparent text-muted-foreground hover:border-sidebar-border/70 hover:bg-background/70 hover:text-foreground',
            )
          "
          @click="void activateRailItem('settings')"
        >
          <component :is="settingsNavItem.icon" class="h-5 w-5" />
        </button>
      </div>
    </div>

    <div
      class="min-w-0 flex-1 bg-sidebar"
      :class="
        props.expanded && !props.resizing
          ? 'flex flex-col overflow-hidden'
          : 'pointer-events-none h-0 overflow-hidden opacity-0'
      "
      :aria-hidden="!props.expanded || props.resizing"
    >
      <div v-if="activeTab === 'chat'" class="min-h-0 flex-1 overflow-hidden">
        <ChatSidebar class="h-full" />
      </div>

      <div v-else class="flex h-full min-h-0 flex-1 items-center justify-center px-4 text-xs text-muted-foreground">
        <span v-if="activeTab === 'hub'">{{ t('hub.title') }}</span>
        <span v-else>{{ t('nav.settings') }}</span>
      </div>
    </div>
  </div>
</template>
