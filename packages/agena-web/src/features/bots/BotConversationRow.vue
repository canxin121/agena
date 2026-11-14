<script setup lang="ts">
import { computed, type ComponentPublicInstance } from 'vue'
import { RiChat3Line, RiHistoryLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import { formatTime } from '@/features/sessions/model/labels'
import type { BotConversation } from '@/features/bots/useBotConversations'
import type { MenuRefEl } from '@/layout/chatSidebar/sessionMenu.types'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import SidebarSessionActionMenu from '@/layout/chatSidebar/components/SidebarSessionActionMenu.vue'

const props = defineProps<{
  botName: string
  conversation: BotConversation
  selectedSessionId: string | null
  uiIsCompactLayout: boolean
  menuOpen: boolean
  menuQuery: string
  menuItems: SessionActionItem[]
  menuAnchorEl: HTMLElement | null
  setMenuRef: (el: MenuRefEl) => void
}>()

const emit = defineEmits<{
  (e: 'open', conversation: BotConversation): void
  (e: 'open-action-menu', conversation: BotConversation, event: MouseEvent | PointerEvent): void
  (e: 'update:menuQuery', value: string): void
  (e: 'select', item: SessionActionItem): void
}>()

const { t } = useI18n()
const subtitle = computed(() => {
  if (props.conversation.localHistory) return t('bot.conversations.archivedHistory')
  if (props.conversation.state === 'running') return t('bot.conversations.running')
  if (props.conversation.state === 'awaiting_interaction') return t('bot.conversations.awaiting')
  return props.conversation.messageCount > 0
    ? t('bot.conversations.messages', { count: props.conversation.messageCount })
    : t('bot.conversations.empty')
})

function setMenuRef(el: Element | ComponentPublicInstance | null) {
  props.setMenuRef(el)
}
</script>

<template>
  <li class="group relative" :data-sidebar-bot-conversation-id="conversation.id">
    <SidebarListItem
      :data-keyboard-item="`bot-conversation:${conversation.id}`"
      :data-keyboard-label="`${botName} · ${conversation.title}`"
      :data-keyboard-level="conversation.localHistory ? 4 : 3"
      :data-keyboard-selected="String(selectedSessionId === conversation.id)"
      :active="selectedSessionId === conversation.id"
      :actions-always-visible="uiIsCompactLayout"
      class="relative gap-2"
      @click="emit('open', conversation)"
    >
      <template #icon>
        <component
          :is="conversation.localHistory ? RiHistoryLine : RiChat3Line"
          class="h-4 w-4 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
      </template>
      <div class="flex w-full min-w-0 items-center gap-2">
        <div class="flex min-w-0 flex-1 flex-col justify-center">
          <span dir="auto" class="min-w-0 truncate text-left typography-ui-label">{{ conversation.title }}</span>
          <span class="w-full truncate text-left text-[10px] text-muted-foreground/70">{{ subtitle }}</span>
        </div>
        <span class="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground/60">
          {{ formatTime(conversation.lastMessageAtMs || conversation.updatedAtMs) }}
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
      v-if="menuOpen"
      :open="true"
      :query="menuQuery"
      :items="menuItems"
      :set-menu-ref="setMenuRef"
      :anchor-el="menuAnchorEl"
      :desktop-placement="uiIsCompactLayout ? 'top-end' : 'bottom-end'"
      @update:query="(value) => emit('update:menuQuery', value)"
      @select="(item) => emit('select', item)"
    />
  </li>
</template>
