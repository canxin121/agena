<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import BotSidebarHeader from '@/features/bots/BotSidebarHeader.vue'
import BotSidebarRow from '@/features/bots/BotSidebarRow.vue'
import { searchBots } from '@/features/bots/profiles'
import { useBotConversationActions } from '@/features/bots/useBotConversationActions'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { copyTextToClipboard } from '@/lib/clipboard'
import type { BotProfile } from '@/lib/bots'
import SidebarPager from '@/layout/chatSidebar/components/SidebarPager.vue'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'
import {
  buildSessionActionItemsForSessionI18n,
  useSessionActionMenu,
  type SessionActionItem,
} from '@/layout/chatSidebar/useSessionActionMenu'
import { useBotsStore } from '@/stores/bots'
import { useChatStore } from '@/stores/chat'
import { useSettingsStore } from '@/stores/settings'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'

const PAGE_SIZE = 12

const bots = useBotsStore()
const chat = useChatStore()
const settings = useSettingsStore()
const toasts = useToastsStore()
const ui = useUiStore()
const navigation = useWorkspaceNavigation()
const route = useRoute()
const { t } = useI18n()

const query = ref('')
const page = ref(0)

const matched = computed<BotProfile[]>(() => searchBots(bots.profiles, query.value))
const total = computed(() => matched.value.length)
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const pageRows = computed(() => matched.value.slice(page.value * PAGE_SIZE, (page.value + 1) * PAGE_SIZE))

watch(total, () => {
  const maxPage = Math.max(0, pageCount.value - 1)
  if (page.value > maxPage) page.value = maxPage
})

// Searching replaces the rows the menu is anchored to, so close it the same way
// the project sidebar does instead of leaving a detached panel.
watch(query, () => {
  page.value = 0
  menu.sessionActionMenuTarget.value = null
  menu.sessionActionMenuAnchorRef.value = null
})

function isSelected(profile: BotProfile): boolean {
  return route.path === '/bot' && route.query.botId === profile.id
}

function open(id?: string, name?: string) {
  ui.setSessionSwitcherOpen(false)
  if (!id) {
    void navigation.openMainTab('bot')
    return
  }
  void navigation.openWorkspaceLocation('bot', { query: { botId: id }, title: name, matchKeys: ['botId'] })
}

// A Bot row's conversation is the pane the row opens, so every row menu item
// acts on it. The project sidebar routes the same items through the focused
// chat page; here the row owns the exact identity, so it runs them directly.
const conversationActions = useBotConversationActions({
  chat: {
    getSessionById: (id) => {
      const session = chat.getSessionById(id)
      return session
        ? {
            id: session.id,
            title: session.title,
            time: session.created_at ? { created: Date.parse(session.created_at) } : undefined,
          }
        : null
    },
    renameSession: (id, title) => chat.renameSession(id, title),
  },
  toasts,
  showThinking: () => settings.data?.showReasoningTraces !== false,
  copyToClipboard: async (text) => {
    await copyTextToClipboard(text)
  },
})

// The shared menu owns its open/close lifecycle; a Bot row supplies the action
// handler and keeps every conversation-level item acting on that row's session.
const menu = useSessionActionMenu({
  chat: { selectedSessionId: null },
  ui: { requestSessionAction: () => {} },
  selectSession: async () => {},
  runAction: (item, target) => runMenuAction(item, target),
})

const activeProfile = computed(
  () => bots.profiles.find((profile) => profile.id === (menu.sessionActionMenuTarget.value?.session?.id || '')) || null,
)

const menuItems = computed<SessionActionItem[]>(() => {
  const profile = activeProfile.value
  if (!profile) return []
  const session = chat.getSessionById(String(profile.session_id))
  return buildSessionActionItemsForSessionI18n(
    t,
    { id: profile.id, favorite: session?.favorite === true },
    { bot: true },
  )
})

function openMenu(profile: BotProfile, event: MouseEvent | PointerEvent) {
  menu.openSessionActionMenu(null, { id: profile.id }, event)
}

async function runMenuAction(item: SessionActionItem, target: { session: { id: string } }): Promise<boolean> {
  const profile = bots.profiles.find((candidate) => candidate.id === target.session.id)
  if (!profile) return true
  if (item.id === 'edit-bot') {
    bots.open('edit', profile.id)
    return true
  }
  if (item.id === 'archive-bot') {
    bots.open('archive', profile.id)
    return true
  }
  if (item.id === 'rename') {
    bots.open('rename', profile.id)
    return true
  }
  if (item.id === 'copy-transcript') {
    await conversationActions.copyTranscript(String(profile.session_id))
    return true
  }
  if (item.id === 'export-transcript') {
    await conversationActions.exportTranscript(String(profile.session_id))
    return true
  }
  if (item.id === 'toggle-favorite') {
    await toggleFavorite(profile.session_id)
    return true
  }
  return true
}

async function toggleFavorite(sessionId: number) {
  const sid = String(sessionId)
  const current = chat.getSessionById(sid)?.favorite === true
  try {
    await chat.updateSessionMetadata(sid, { favorite: !current })
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  }
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
              :menu-open="menu.sessionActionMenuTarget.value?.session?.id === profile.id"
              :menu-query="menu.sessionActionMenuQuery.value"
              :menu-items="menuItems"
              :menu-anchor-el="menu.sessionActionMenuAnchorRef.value"
              :set-menu-ref="menu.setSessionActionMenuRef"
              :menu-placement="ui.isCompactLayout ? 'top-end' : 'bottom-end'"
              @open="open(profile.id, profile.name)"
              @open-action-menu="(event) => openMenu(profile, event)"
              @update:menu-query="(v) => (menu.sessionActionMenuQuery.value = v)"
              @select="(item) => menu.runSessionActionMenu(item)"
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
