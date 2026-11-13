<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import BotSidebarHeader from '@/features/bots/BotSidebarHeader.vue'
import BotSidebarRow from '@/features/bots/BotSidebarRow.vue'
import BotConversationGroup from '@/features/bots/BotConversationGroup.vue'
import { searchBots } from '@/features/bots/profiles'
import { useBotConversationActions } from '@/features/bots/useBotConversationActions'
import { useBotConversations, type BotConversation } from '@/features/bots/useBotConversations'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { copyTextToClipboard } from '@/lib/clipboard'
import { getLocalString, setLocalString } from '@/lib/persist'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import IconButton from '@/components/ui/IconButton.vue'
import { RiAddLine, RiDeleteBinLine } from '@remixicon/vue'
import { botIdQuery, createBotConversation, type BotProfile } from '@/lib/bots'
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
import type { SessionOwnerSurface } from '@/types/chat'

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
// The bot list is paged; remember the page so reopening the sidebar lands where
// the user left it instead of jumping back to the first one.
const PAGE_STORAGE_KEY = localStorageKeys.ui.botSidebarPage
const page = ref(Math.max(0, Number.parseInt(getLocalString(PAGE_STORAGE_KEY).trim(), 10) || 0))
watch(page, (value) => setLocalString(PAGE_STORAGE_KEY, String(Math.max(0, Math.floor(Number(value) || 0)))))

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

// The Bot whose conversations the sidebar lists: the one the route selected,
// falling back to the first active Bot so the section is never empty on arrival.
const selectedProfile = computed<BotProfile | null>(() => {
  const requested = botIdQuery(route.query)
  if (requested && !requested.startsWith('!')) {
    const match = bots.profiles.find((profile) => profile.id === requested && !profile.archived)
    if (match) return match
  }
  return bots.profiles.find((profile) => !profile.archived) || null
})

const conversations = useBotConversations()

// Which surface groups are expanded. They default open: a Bot usually has one
// local conversation plus whatever rooms it serves, and collapsing those would
// hide the very thing this section exists to show.
const openSurfaces = ref<Record<SessionOwnerSurface, boolean>>({
  local: true,
  direct: true,
  group: true,
  broadcast: true,
})

watch(
  () => selectedProfile.value?.id,
  (id) => {
    menu.sessionActionMenuTarget.value = null
    menu.sessionActionMenuAnchorRef.value = null
    if (id) void conversations.load(id)
    else conversations.reset()
  },
  { immediate: true },
)

const conversationMenuTarget = ref<BotConversation | null>(null)

const conversationMenuItems = computed<SessionActionItem[]>(() => {
  const conversation = conversationMenuTarget.value
  if (!conversation) return []
  const busy = Boolean(conversation.running)
  // A conversation row acts on its own transcript, so it gets the project
  // row's conversation actions plus an explicit, confirmed delete. It has no
  // Bot profile to edit: the row above owns that identity.
  return [
    ...buildSessionActionItemsForSessionI18n(
      t,
      { id: conversation.id, favorite: conversation.favorite },
      { bot: true, fork: false },
    ),
    // A conversation with live work is stopped before it is deleted: the
    // menu says so up front, and confirming performs both steps. Without live
    // work the plain delete wording is accurate.
    {
      id: 'delete-conversation',
      label: String(
        t(
          busy
            ? 'bot.conversations.deleteBusy'
            : 'bot.conversations.delete',
        ),
      ),
      description: String(
        t(busy ? 'bot.conversations.deleteBusyHint' : 'bot.conversations.deleteHint'),
      ),
      icon: RiDeleteBinLine,
      variant: 'destructive' as const,
      confirmTitle: String(
        t(busy ? 'bot.conversations.deleteBusy' : 'bot.conversations.delete'),
      ),
      confirmDescription: String(
        t(busy ? 'bot.conversations.deleteBusyHint' : 'bot.conversations.deleteHint'),
      ),
      confirmText: String(t(busy ? 'bot.conversations.deleteBusy' : 'common.delete')),
      cancelText: String(t('common.cancel')),
    },
  ]
})

function openConversationMenu(conversation: BotConversation) {
  conversationMenuTarget.value = conversation
}

function runConversationAction(item: SessionActionItem): void {
  const conversation = conversationMenuTarget.value
  if (!conversation) return
  conversationMenuTarget.value = null
  if (item.id === 'toggle-favorite') {
    void toggleFavorite(conversation)
    return
  }
  if (item.id === 'copy-transcript') {
    void conversationActions.copyTranscript(conversation.id)
    return
  }
  if (item.id === 'export-transcript') {
    void conversationActions.exportTranscript(conversation.id)
    return
  }
  if (item.id === 'rename') {
    void renameConversation(conversation)
    return
  }
  if (item.id === 'delete-conversation') {
    void chat
      .deleteSession(conversation.id, { stopFirst: conversation.running })
      .then(() => conversations.load(String(selectedProfile.value?.id || ''), true))
      .catch((error: unknown) => toasts.push('error', error instanceof Error ? error.message : String(error)))
  }
}

async function renameConversation(conversation: BotConversation): Promise<void> {
  const title = window.prompt(String(t('bot.conversations.renamePrompt')), conversation.title)?.trim()
  if (!title || title === conversation.title) return
  try {
    await chat.renameSession(conversation.id, title)
    await conversations.load(String(selectedProfile.value?.id || ''), true)
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  }
}

/**
 * Start an empty conversation for the selected Bot and open it.
 *
 * Unlike opening the Bot (which reopens the latest conversation), this always
 * creates a new one, so several topics can coexist without deleting history.
 */
async function startConversation() {
  const profile = selectedProfile.value
  if (!profile) return
  try {
    const session = await createBotConversation(profile.id)
    await conversations.load(profile.id, true)
    openConversation({
      id: String(session.session_id),
      title: session.name,
      surface: 'local',
      favorite: false,
      pinned: false,
      updatedAtMs: 0,
      lastMessageAtMs: 0,
      messageCount: 0,
    })
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  }
}

function openConversation(conversation: BotConversation) {
  ui.setSessionSwitcherOpen(false)
  void navigation.openWorkspaceLocation('bot', {
    query: { botId: selectedProfile.value?.id, sessionId: conversation.id },
    title: conversation.title,
    matchKeys: ['botId', 'sessionId'],
  })
}

const menuItems = computed<SessionActionItem[]>(() => {
  const profile = activeProfile.value
  if (!profile) return []
  return buildSessionActionItemsForSessionI18n(
    t,
    { id: profile.id, favorite: false },
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
  // A Bot row is an identity, not a conversation. The conversation-level
  // actions live on the rows under it, where they have a real session id.
  return true
}

async function toggleFavorite(conversation: BotConversation) {
  try {
    await chat.updateSessionMetadata(conversation.id, { favorite: !conversation.favorite })
    await conversations.load(String(selectedProfile.value?.id || ''), true)
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

        <!--
          Conversations of the selected Bot, grouped by the entrance shape the
          server recorded. This is the same sidebar row primitive the project
          sidebar uses; only the data source differs, because these
          conversations have no project and are found by owner instead.
        -->
        <div
          v-if="selectedProfile"
          class="mt-3 border-t border-sidebar-border/60 pt-2"
          :aria-label="String(t('bot.conversations.title', { name: selectedProfile.name }))"
        >
          <div class="flex items-center justify-between gap-2 px-1.5 pb-1">
            <span class="truncate typography-ui-label font-medium text-muted-foreground">
              {{ t('bot.conversations.title', { name: selectedProfile.name }) }}
            </span>
            <div class="flex shrink-0 items-center gap-1">
              <span class="font-mono text-[11px] text-muted-foreground/70">{{ conversations.total.value }}</span>
              <IconButton
                data-testid="sidebar-bot-new-conversation"
                :tooltip="String(t('bot.conversations.create'))"
                :title="String(t('bot.conversations.create'))"
                :aria-label="String(t('bot.conversations.create'))"
                :is-touch-pointer="ui.isTouchPointer || ui.isCompactTouch"
                @click="startConversation"
              >
                <RiAddLine class="h-3.5 w-3.5" />
              </IconButton>
            </div>
          </div>

          <SidebarSectionSkeleton v-if="conversations.loading.value && !conversations.total.value" :rows="2" compact />

          <p
            v-else-if="conversations.error.value"
            role="alert"
            class="break-words px-1.5 py-2 text-xs text-destructive"
          >
            {{ conversations.error.value }}
            <button class="underline" @click="conversations.load(selectedProfile.id, true)">
              {{ t('common.retry') }}
            </button>
          </p>

          <template v-else>
            <BotConversationGroup
              v-for="group in conversations.groups.value"
              :key="group.surface"
              :bot-name="selectedProfile.name"
              :group="group"
              :open="openSurfaces[group.surface]"
              :selected-session-id="String(route.query.sessionId || '')"
              :ui-is-compact-layout="ui.isCompactLayout"
              :menu-open-for="(id) => conversationMenuTarget?.id === id"
              :menu-query="menu.sessionActionMenuQuery.value"
              :menu-items-for="() => conversationMenuItems"
              :menu-anchor-el="menu.sessionActionMenuAnchorRef.value"
              :set-menu-ref="menu.setSessionActionMenuRef"
              @update:open="(v) => (openSurfaces[group.surface] = v)"
              @open="openConversation"
              @open-action-menu="
                (conversation, event) => {
                  menu.sessionActionMenuAnchorRef.value = event.currentTarget as HTMLElement
                  openConversationMenu(conversation)
                  event.preventDefault()
                  event.stopPropagation()
                }
              "
              @update:menu-query="(v) => (menu.sessionActionMenuQuery.value = v)"
              @select="runConversationAction"
            />
          </template>
        </div>
      </div>
    </div>
  </aside>
</template>
