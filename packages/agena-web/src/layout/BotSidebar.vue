<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { RiDeleteBinLine } from '@remixicon/vue'

import BotSidebarHeader from '@/features/bots/BotSidebarHeader.vue'
import BotSidebarRow from '@/features/bots/BotSidebarRow.vue'
import BotConversationGroup from '@/features/bots/BotConversationGroup.vue'
import { searchBots } from '@/features/bots/profiles'
import { useBotConversationActions } from '@/features/bots/useBotConversationActions'
import { useBotConversations, type BotConversation } from '@/features/bots/useBotConversations'
import { useBotSidebarState } from '@/features/bots/useBotSidebarState'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { copyTextToClipboard } from '@/lib/clipboard'
import { getLocalString, setLocalString } from '@/lib/persist'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { createRevalidator } from '@/lib/revalidation'
import { subscribeResource } from '@/lib/resourceSync'
import { isDocumentVisible } from '@/lib/backgroundReads'
import { botIdQuery, type BotProfile } from '@/lib/bots'
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
const conversations = useBotConversations()
const tree = useBotSidebarState()

const query = ref('')
const PAGE_STORAGE_KEY = localStorageKeys.ui.botSidebarPage
const page = ref(Math.max(0, Number.parseInt(getLocalString(PAGE_STORAGE_KEY).trim(), 10) || 0))
watch(page, (value) => setLocalString(PAGE_STORAGE_KEY, String(Math.max(0, Math.floor(Number(value) || 0)))))

const matched = computed<BotProfile[]>(() => searchBots(bots.profiles, query.value))
const total = computed(() => matched.value.length)
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const pageRows = computed(() => matched.value.slice(page.value * PAGE_SIZE, (page.value + 1) * PAGE_SIZE))
const selectedBotId = computed(() => (route.path === '/bot' ? botIdQuery(route.query) : ''))
const selectedSessionId = computed(() => (route.path === '/bot' ? String(route.query.sessionId || '') : ''))

watch(total, () => {
  if (page.value >= pageCount.value) page.value = pageCount.value - 1
})

type ConversationTarget = { botId: string; conversation: BotConversation }
const conversationMenuTarget = ref<ConversationTarget | null>(null)
const menu = useSessionActionMenu({
  chat: { selectedSessionId: null },
  ui: { requestSessionAction: () => {} },
  selectSession: async () => {},
  runAction: (item, target) => runMenuAction(item, target),
})

function closeMenu() {
  menu.sessionActionMenuTarget.value = null
  menu.sessionActionMenuAnchorRef.value = null
  conversationMenuTarget.value = null
}
watch(query, () => {
  page.value = 0
  closeMenu()
})
watch(page, closeMenu)

function openLocal(profile: BotProfile) {
  ui.setSessionSwitcherOpen(false)
  tree.setBotOpen(profile.id, true)
  tree.setGroupOpen(profile.id, 'local', true)
  void navigation.openWorkspaceLocation('bot', {
    query: { botId: profile.id, ...(profile.local_session_id ? { sessionId: String(profile.local_session_id) } : {}) },
    title: profile.name,
    matchKeys: ['botId', 'sessionId'],
  })
}

function openConversation(profile: BotProfile, conversation: BotConversation) {
  ui.setSessionSwitcherOpen(false)
  void navigation.openWorkspaceLocation('bot', {
    query: { botId: profile.id, sessionId: conversation.id },
    title: conversation.title,
    matchKeys: ['botId', 'sessionId'],
  })
}

function toggleBot(profile: BotProfile) {
  const open = !tree.isBotOpen(profile.id)
  tree.setBotOpen(profile.id, open)
  closeMenu()
  if (open) void conversations.load(profile.id, profile.local_session_id)
}

async function reloadConversations(botId: string) {
  const profile = bots.profiles.find((item) => item.id === botId)
  if (profile) await conversations.load(botId, profile.local_session_id, true)
}

async function refresh() {
  await bots.refresh(true)
  await refreshExpanded()
}

async function refreshExpanded() {
  await Promise.all(
    pageRows.value.filter((profile) => tree.isBotOpen(profile.id)).map((profile) => reloadConversations(profile.id)),
  )
}

const liveConversations = createRevalidator(refreshExpanded, { intervalMs: 500, enabled: isDocumentVisible })
const releaseSessions = subscribeResource('sessions', () => liveConversations.invalidate())
watch(
  () => bots.authoritative,
  () => liveConversations.invalidate(),
)

// Each expanded Bot owns its request and pagination state; changing the current
// conversation never replaces another Bot's children.
watch(
  () =>
    pageRows.value
      .filter((profile) => tree.isBotOpen(profile.id))
      .map((profile) => [profile.id, profile.local_session_id] as const),
  (profiles) => {
    for (const [id, localSessionId] of profiles) void conversations.load(id, localSessionId)
  },
  { immediate: true },
)

let revealedRoute = ''
watch(
  () => [selectedBotId.value, selectedSessionId.value, bots.profiles] as const,
  async ([id, sessionId]) => {
    const profile = bots.profiles.find((item) => item.id === id)
    const identity = id + ':' + sessionId
    if (!profile || identity === revealedRoute) return
    const initial = revealedRoute === ''
    revealedRoute = identity
    if (!initial || !tree.hasBotState(id)) tree.setBotOpen(id, true)
    if (!query.value.trim()) {
      const index = matched.value.findIndex((item) => item.id === id)
      if (index >= 0) page.value = Math.floor(index / PAGE_SIZE)
    }
    await conversations.load(id, profile.local_session_id, true)
    if (identity !== revealedRoute) return
    for (const group of conversations.groups(id)) {
      const conversation = [...group.conversations, ...group.history].find((item) => item.id === sessionId)
      if (!conversation) continue
      if (!initial || !tree.hasGroupState(id, group.surface)) tree.setGroupOpen(id, group.surface, true)
      if (conversation.localHistory && (!initial || !tree.hasGroupState(id, 'local-history')))
        tree.setGroupOpen(id, 'local-history', true)
    }
  },
  { immediate: true },
)
watch(
  () => bots.version,
  () => {
    closeMenu()
    conversations.reset()
    revealedRoute = ''
  },
  { flush: 'sync' },
)
onBeforeUnmount(() => {
  releaseSessions()
  liveConversations.dispose()
  conversations.reset()
})

const conversationActions = useBotConversationActions({
  chat: {
    getSessionById: (id) => {
      const session = chat.getSessionById(id)
      if (session) {
        return {
          id: session.id,
          title: session.title,
          time: session.created_at ? { created: Date.parse(session.created_at) } : undefined,
        }
      }
      // Export also works for a row whose transcript has not been opened yet.
      for (const profile of bots.profiles) {
        for (const group of conversations.groups(profile.id)) {
          const item = [...group.conversations, ...group.history].find((candidate) => candidate.id === id)
          if (item) return { id, title: item.title }
        }
      }
      return null
    },
    renameSession: (id, title) => chat.renameSession(id, title),
  },
  toasts,
  showThinking: () => settings.data?.showReasoningTraces !== false,
  copyToClipboard: async (text) => {
    await copyTextToClipboard(text)
  },
})

const menuItems = computed<SessionActionItem[]>(() => {
  const targetId = menu.sessionActionMenuTarget.value?.session.id
  const target = conversationMenuTarget.value
  let items: SessionActionItem[]
  if (target && target.conversation.id === targetId) {
    const conversation = target.conversation
    items = buildSessionActionItemsForSessionI18n(t, conversation, { fork: false })
    if (conversation.localHistory) {
      items = items.filter((item) => item.id === 'copy-transcript' || item.id === 'export-transcript')
    } else {
      const busy = conversation.running
      items.push({
        id: 'delete-conversation',
        label: String(t(busy ? 'bot.conversations.deleteBusy' : 'bot.conversations.delete')),
        description: String(t(busy ? 'bot.conversations.deleteBusyHint' : 'bot.conversations.deleteHint')),
        icon: RiDeleteBinLine,
        variant: 'destructive',
        confirmTitle: String(t(busy ? 'bot.conversations.deleteBusy' : 'bot.conversations.delete')),
        confirmDescription: String(t(busy ? 'bot.conversations.deleteBusyHint' : 'bot.conversations.deleteHint')),
        confirmText: String(t(busy ? 'bot.conversations.deleteBusy' : 'common.delete')),
        cancelText: String(t('common.cancel')),
      })
    }
  } else {
    const profile = bots.profiles.find((item) => item.id === targetId)
    items = profile
      ? buildSessionActionItemsForSessionI18n(t, { id: profile.id }, { bot: true }).filter((item) =>
          ['edit-bot', 'archive-bot', 'rename'].includes(item.id),
        )
      : []
  }
  const search = menu.sessionActionMenuQuery.value.trim().toLowerCase()
  return search
    ? items.filter((item) => (item.label + ' ' + (item.description || '')).toLowerCase().includes(search))
    : items
})

function openProfileMenu(profile: BotProfile, event: MouseEvent | PointerEvent) {
  conversationMenuTarget.value = null
  menu.openSessionActionMenu(null, { id: profile.id }, event)
}

function openConversationMenu(profile: BotProfile, conversation: BotConversation, event: MouseEvent | PointerEvent) {
  conversationMenuTarget.value = { botId: profile.id, conversation }
  menu.openSessionActionMenu(null, conversation, event)
}

async function runMenuAction(item: SessionActionItem, target: { session: { id: string } }): Promise<boolean> {
  const selected = conversationMenuTarget.value
  conversationMenuTarget.value = null
  if (selected && selected.conversation.id === target.session.id) {
    const { botId, conversation } = selected
    try {
      if (item.id === 'copy-transcript') await conversationActions.copyTranscript(conversation.id)
      else if (item.id === 'export-transcript') await conversationActions.exportTranscript(conversation.id)
      else if (!conversation.localHistory) {
        if (item.id === 'toggle-favorite') {
          await chat.updateSessionMetadata(conversation.id, { favorite: !conversation.favorite })
        } else if (item.id === 'rename') {
          const title = window.prompt(String(t('bot.conversations.renamePrompt')), conversation.title)?.trim()
          if (!title || title === conversation.title) return true
          await chat.renameSession(conversation.id, title)
        } else if (item.id === 'delete-conversation') {
          await chat.deleteSession(conversation.id, { stopFirst: conversation.running })
          await bots.refresh(true)
        }
        await reloadConversations(botId)
      }
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : String(error))
    }
    return true
  }
  const profile = bots.profiles.find((candidate) => candidate.id === target.session.id)
  if (!profile) return true
  if (item.id === 'edit-bot') bots.open('edit', profile.id)
  else if (item.id === 'archive-bot') bots.open('archive', profile.id)
  else if (item.id === 'rename') bots.open('rename', profile.id)
  return true
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
      @update:query="(value) => (query = value)"
      @create="bots.open('create')"
      @refresh="refresh"
    />
    <div v-keyboard-collection="{ tree: true }" class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-2">
      <div class="flex min-h-full flex-col px-2 pt-1">
        <SidebarSectionSkeleton v-if="bots.loading && !bots.profiles.length" :rows="3" compact />
        <div v-else-if="bots.error" role="alert" class="break-words px-1.5 py-2 text-xs text-destructive">
          {{ bots.error }}
          <button class="underline" @click="refresh">{{ t('common.retry') }}</button>
        </div>
        <p v-else-if="!total" data-testid="sidebar-bots" class="px-1.5 py-2 text-xs text-muted-foreground">
          {{ query.trim() ? t('bot.profiles.noMatches') : t('bot.profiles.empty') }}
        </p>
        <ul v-else data-testid="sidebar-bots" class="space-y-1" :aria-label="String(t('bot.title'))">
          <li v-for="profile in pageRows" :key="profile.id" :data-sidebar-bot-id="profile.id">
            <BotSidebarRow
              :profile="profile"
              :selected="selectedBotId === profile.id"
              :collapsed="!tree.isBotOpen(profile.id)"
              :ui-is-compact-layout="ui.isCompactLayout"
              :busy="bots.loading"
              :menu-open="menu.sessionActionMenuTarget.value?.session.id === profile.id"
              :menu-query="menu.sessionActionMenuQuery.value"
              :menu-items="menuItems"
              :menu-anchor-el="menu.sessionActionMenuAnchorRef.value"
              :set-menu-ref="menu.setSessionActionMenuRef"
              :menu-placement="ui.isCompactLayout ? 'top-end' : 'bottom-end'"
              @open="openLocal(profile)"
              @toggle-collapse="toggleBot(profile)"
              @open-action-menu="(event) => openProfileMenu(profile, event)"
              @update:menu-query="(value) => (menu.sessionActionMenuQuery.value = value)"
              @select="(item) => menu.runSessionActionMenu(item)"
            />
            <div v-if="tree.isBotOpen(profile.id)" class="space-y-0.5 pb-1 pl-3">
              <BotConversationGroup
                v-for="group in conversations.groups(profile.id)"
                :key="group.surface"
                :bot-id="profile.id"
                :bot-name="profile.name"
                :group="group"
                :open="tree.isGroupOpen(profile.id, group.surface)"
                :history-open="tree.isGroupOpen(profile.id, 'local-history')"
                :selected-session-id="selectedSessionId"
                :ui-is-compact-layout="ui.isCompactLayout"
                :menu-open-for="(id) => menu.sessionActionMenuTarget.value?.session.id === id"
                :menu-query="menu.sessionActionMenuQuery.value"
                :menu-items="menuItems"
                :menu-anchor-el="menu.sessionActionMenuAnchorRef.value"
                :set-menu-ref="menu.setSessionActionMenuRef"
                @update:open="
                  (open) => {
                    tree.setGroupOpen(profile.id, group.surface, open)
                    closeMenu()
                  }
                "
                @update:history-open="
                  (open) => {
                    tree.setGroupOpen(profile.id, 'local-history', open)
                    closeMenu()
                  }
                "
                @open="(conversation) => openConversation(profile, conversation)"
                @open-local="openLocal(profile)"
                @load-more="conversations.loadMore(profile.id, group.surface)"
                @retry="conversations.reloadGroup(profile.id, group.surface)"
                @open-action-menu="(conversation, event) => openConversationMenu(profile, conversation, event)"
                @update:menu-query="(value) => (menu.sessionActionMenuQuery.value = value)"
                @select="(item) => menu.runSessionActionMenu(item)"
              />
            </div>
          </li>
        </ul>
        <SidebarPager
          v-if="pageCount > 1"
          :page="page"
          :page-count="pageCount"
          :prev-label="String(t('chat.sidebar.directoriesList.prevSessionsPage'))"
          :next-label="String(t('chat.sidebar.directoriesList.nextSessionsPage'))"
          @update:page="(value) => (page = value)"
        />
      </div>
    </div>
  </aside>
</template>
