<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import BotSidebarHeader from '@/features/bots/BotSidebarHeader.vue'
import BotSidebarRow from '@/features/bots/BotSidebarRow.vue'
import { searchBots } from '@/features/bots/profiles'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import type { BotProfile } from '@/lib/bots'
import SidebarPager from '@/layout/chatSidebar/components/SidebarPager.vue'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'
import { useBotsStore } from '@/stores/bots'
import { useUiStore } from '@/stores/ui'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'

const PAGE_SIZE = 12

const bots = useBotsStore()
const ui = useUiStore()
const navigation = useWorkspaceNavigation()
const route = useRoute()
const { t } = useI18n()

const query = ref('')
const page = ref(0)
const menuOpenId = ref('')

function closeMenus(event: PointerEvent) {
  if (!menuOpenId.value) return
  const menuPanels = document.querySelectorAll('[data-keyboard-overlay="true"]')
  if (event.target instanceof Element) {
    if (event.target.closest('[data-testid="sidebar-bot-actions"]')) return
    for (const panel of menuPanels) if (panel.contains(event.target)) return
  }
  menuOpenId.value = ''
}

onMounted(() => document.addEventListener('pointerdown', closeMenus))
onBeforeUnmount(() => document.removeEventListener('pointerdown', closeMenus))

const matched = computed<BotProfile[]>(() => searchBots(bots.profiles, query.value))
const total = computed(() => matched.value.length)
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const pageRows = computed(() => matched.value.slice(page.value * PAGE_SIZE, (page.value + 1) * PAGE_SIZE))

watch(total, () => {
  const maxPage = Math.max(0, pageCount.value - 1)
  if (page.value > maxPage) page.value = maxPage
})

watch(query, () => {
  page.value = 0
  menuOpenId.value = ''
})

function isSelected(profile: BotProfile): boolean {
  return route.path === '/bot' && route.query.botId === profile.id
}

function open(id?: string, name?: string) {
  ui.setSessionSwitcherOpen(false)
  menuOpenId.value = ''
  if (!id) {
    void navigation.openMainTab('bot')
    return
  }
  void navigation.openWorkspaceLocation('bot', { query: { botId: id }, title: name, matchKeys: ['botId'] })
}

function runAction(action: 'edit' | 'rename' | 'refresh' | 'archive', profile: BotProfile) {
  menuOpenId.value = ''
  if (action === 'refresh') {
    void bots.refresh(true)
    return
  }
  bots.open(action, profile.id)
}
</script>

<template>
  <aside
    data-testid="bot-sidebar"
    class="oc-bot-sidebar flex h-full min-h-0 flex-col overflow-hidden bg-sidebar"
    :aria-label="String(t('bot.title'))"
  >
    <BotSidebarHeader
      :query="query"
      :loading="bots.loading"
      :is-touch-pointer="ui.isTouchPointer"
      :is-compact-touch="ui.isCompactTouch"
      @update:query="(v) => (query = v)"
      @create="bots.open('create')"
      @refresh="bots.refresh(true)"
    />

    <div v-keyboard-collection="{ tree: true }" class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-2">
      <div class="flex min-h-full flex-col px-2 pt-1">
        <SidebarSectionSkeleton v-if="bots.loading && !bots.profiles.length" :rows="3" compact />

        <div v-else-if="bots.error" role="alert" class="break-words px-1.5 py-2 text-xs text-destructive">
          {{ bots.error }}
          <button class="underline" @click="bots.refresh(true)">{{ t('common.retry') }}</button>
        </div>

        <p v-else-if="!total" data-testid="sidebar-bots" class="px-1.5 py-2 text-xs text-muted-foreground">
          {{ query.trim() ? t('bot.profiles.noMatches') : t('bot.profiles.empty') }}
        </p>

        <ul v-else data-testid="sidebar-bots" class="space-y-1" :aria-label="String(t('bot.title'))">
          <li v-for="profile in pageRows" :key="profile.id" :data-sidebar-bot-id="profile.id">
            <BotSidebarRow
              :profile="profile"
              :selected="isSelected(profile)"
              :ui-is-compact-layout="ui.isCompactLayout"
              :busy="bots.loading"
              :menu-open="menuOpenId === profile.id"
              @open="open(profile.id, profile.name)"
              @update:menu-open="(v) => (menuOpenId = v ? profile.id : menuOpenId === profile.id ? '' : menuOpenId)"
              @action="(action) => runAction(action, profile)"
            />
          </li>
        </ul>

        <SidebarPager
          v-if="pageCount > 1"
          :page="page"
          :page-count="pageCount"
          :prev-label="String(t('chat.sidebar.directoriesList.prevSessionsPage'))"
          :next-label="String(t('chat.sidebar.directoriesList.nextSessionsPage'))"
          @update:page="(p) => (page = p)"
        />
      </div>
    </div>
  </aside>
</template>
