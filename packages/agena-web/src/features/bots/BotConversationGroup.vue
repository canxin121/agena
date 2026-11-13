<script setup lang="ts">
// One collapsible group of a Bot's conversations, keyed by where they were
// entered from. It borrows the project sidebar's section chrome and row
// primitive so both sidebars read the same way.
import { computed, type ComponentPublicInstance } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiRobot2Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import { formatTime } from '@/features/sessions/model/labels'
import type { BotConversation, BotConversationGroup } from '@/features/bots/useBotConversations'
import type { MenuRefEl } from '@/layout/chatSidebar/sessionMenu.types'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import SidebarSessionActionMenu from '@/layout/chatSidebar/components/SidebarSessionActionMenu.vue'

const props = withDefaults(
  defineProps<{
    botName: string
    group: BotConversationGroup
    open: boolean
    selectedSessionId: string | null
    uiIsCompactLayout: boolean
    menuOpenFor: (conversationId: string) => boolean
    menuQuery: string
    menuItemsFor: (conversation: BotConversation) => SessionActionItem[]
    menuAnchorEl: HTMLElement | null
    setMenuRef: (el: MenuRefEl) => void
  }>(),
  {
    selectedSessionId: null,
    menuQuery: '',
    menuAnchorEl: null,
  },
)

const emit = defineEmits<{
  (e: 'update:open', v: boolean): void
  (e: 'open', conversation: BotConversation): void
  (e: 'open-action-menu', conversation: BotConversation, event: MouseEvent | PointerEvent): void
  (e: 'update:menuQuery', v: string): void
  (e: 'select', item: SessionActionItem): void
}>()

const { t } = useI18n()

const count = computed(() => props.group.conversations.length)
const title = computed(() => t(`bot.conversations.surfaces.${props.group.surface}`))
const emptyHint = computed(() => t(`bot.conversations.surfaceHints.${props.group.surface}`))
const actionsAlwaysVisible = computed(() => props.uiIsCompactLayout)

function subtitle(conversation: BotConversation): string {
  if (conversation.state === 'running') return t('bot.conversations.running')
  if (conversation.state === 'awaiting_interaction') return t('bot.conversations.awaiting')
  return conversation.messageCount > 0
    ? t('bot.conversations.messages', { count: conversation.messageCount })
    : t('bot.conversations.empty')
}

function relativeTime(conversation: BotConversation): string {
  return formatTime(conversation.lastMessageAtMs || conversation.updatedAtMs)
}

function setMenuRef(el: Element | ComponentPublicInstance | null) {
  props.setMenuRef(el)
}
</script>

<template>
  <section
    class="flex-shrink-0"
    :data-testid="`bot-surface-${group.surface}`"
    :aria-label="String(title)"
  >
    <div
      :class="[
        'group h-9 px-3 flex items-center justify-between gap-2 rounded-md transition-colors',
        open ? 'bg-secondary/30' : 'hover:bg-secondary/30 focus-within:bg-secondary/30',
      ]"
    >
      <button
        type="button"
        class="flex-1 h-full min-w-0 flex items-center gap-2 text-left rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
        :aria-expanded="open"
        :aria-label="String(t('bot.conversations.toggle', { group: title }))"
        @click="emit('update:open', !open)"
      >
        <component
          :is="open ? RiArrowDownSLine : RiArrowRightSLine"
          class="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <span class="truncate typography-ui-label font-medium text-muted-foreground">{{ title }}</span>
      </button>
      <span class="shrink-0 font-mono text-[11px] text-muted-foreground/70">{{ count }}</span>
    </div>

    <div v-if="open" class="px-2 pb-2">
      <p v-if="count === 0" class="px-2 py-2 text-xs text-muted-foreground" data-testid="bot-surface-empty">
        {{ emptyHint }}
      </p>
      <ul v-else class="space-y-1">
        <li
          v-for="conversation in group.conversations"
          :key="conversation.id"
          class="group relative"
          :data-sidebar-bot-conversation-id="conversation.id"
        >
          <SidebarListItem
            :data-keyboard-item="`bot-conversation:${conversation.id}`"
            :data-keyboard-label="`${botName} · ${conversation.title}`"
            data-keyboard-level="3"
            :data-keyboard-selected="String(selectedSessionId === conversation.id)"
            :active="selectedSessionId === conversation.id"
            :actions-always-visible="actionsAlwaysVisible"
            class="relative gap-2"
            @click="emit('open', conversation)"
          >
            <template #icon>
              <RiRobot2Line class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            </template>

            <div class="flex w-full min-w-0 items-center gap-2">
              <div class="flex min-w-0 flex-1 flex-col justify-center">
                <span dir="auto" class="min-w-0 truncate text-left typography-ui-label">
                  {{ conversation.title }}
                </span>
                <span class="w-full truncate text-left text-[10px] text-muted-foreground/70">
                  {{ subtitle(conversation) }}
                </span>
              </div>
              <span class="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground/60">
                {{ relativeTime(conversation) }}
              </span>
            </div>

            <template #actions>
              <ListItemOverflowActionButton
                :data-testid="`sidebar-bot-conversation-actions-${conversation.id}`"
                :mobile="uiIsCompactLayout"
                :label="String(t('bot.conversations.actions', { name: conversation.title }))"
                @trigger="(event) => emit('open-action-menu', conversation, event)"
              />
            </template>
          </SidebarListItem>

          <SidebarSessionActionMenu
            v-if="menuOpenFor(conversation.id)"
            :open="true"
            :query="menuQuery"
            :items="menuItemsFor(conversation)"
            :set-menu-ref="setMenuRef"
            :anchor-el="menuAnchorEl"
            :desktop-placement="uiIsCompactLayout ? 'top-end' : 'bottom-end'"
            @update:query="(v) => emit('update:menuQuery', v)"
            @select="(item) => emit('select', item)"
          />
        </li>
      </ul>
    </div>
  </section>
</template>
