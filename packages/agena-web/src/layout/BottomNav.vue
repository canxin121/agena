<script setup lang="ts">
import { computed, type Component } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiChat4Line, RiFolder6Line, RiTerminalBoxLine, RiGitMergeLine, RiGlobalLine } from '@remixicon/vue'
import { cn } from '@/lib/utils'
import { MAIN_TABS, mainTabFromPath, type NavigationMainTabId } from '@/app/navigation/mainTabs'

const route = useRoute()
const { t } = useI18n()

const TAB_ICONS: Record<NavigationMainTabId, Component> = {
  chat: RiChat4Line,
  files: RiFolder6Line,
  preview: RiGlobalLine,
  terminal: RiTerminalBoxLine,
  git: RiGitMergeLine,
}

const items = computed(() =>
  MAIN_TABS.map((tab) => ({
    id: tab.id,
    to: tab.path,
    label: String(t(tab.labelKey)),
    icon: TAB_ICONS[tab.id],
  })),
)

const activeMainTab = computed(() => mainTabFromPath(route.path))

function isActive(tab: NavigationMainTabId) {
  return activeMainTab.value === tab
}
</script>

<template>
  <nav
    class="oc-bottom-nav fixed bottom-0 left-0 right-0 z-50 bg-background/80 backdrop-blur-xl border-t border-border pr-[var(--oc-safe-area-right,0px)] pb-[var(--oc-safe-area-bottom,0px)] pl-[var(--oc-safe-area-left,0px)]"
    :aria-label="String(t('aria.primaryNavigation'))"
  >
    <div class="grid grid-cols-5 h-[56px]">
      <RouterLink
        v-for="item in items"
        :key="item.to"
        :to="item.to"
        :class="
          cn(
            'flex flex-col items-center justify-center gap-1 active:scale-95 transition-transform',
            isActive(item.id) ? 'text-primary' : 'text-muted-foreground hover:text-foreground',
          )
        "
        :aria-current="isActive(item.id) ? 'page' : undefined"
      >
        <component :is="item.icon" class="w-5 h-5" />
        <span class="text-[10px] font-medium whitespace-nowrap">{{ item.label }}</span>
      </RouterLink>
    </div>
  </nav>
</template>
