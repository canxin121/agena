<script setup lang="ts">
import { computed } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine, RiChat3Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import SidebarTextButton from '@/components/ui/SidebarTextButton.vue'
import BotConversationRow from '@/features/bots/BotConversationRow.vue'
import type { BotConversation, BotConversationGroup } from '@/features/bots/useBotConversations'
import type { MenuRefEl } from '@/layout/chatSidebar/sessionMenu.types'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'

const props = defineProps<{
  botId: string
  botName: string
  group: BotConversationGroup
  open: boolean
  historyOpen: boolean
  selectedSessionId: string | null
  uiIsCompactLayout: boolean
  menuOpenFor: (conversationId: string) => boolean
  menuQuery: string
  menuItems: SessionActionItem[]
  menuAnchorEl: HTMLElement | null
  setMenuRef: (el: MenuRefEl) => void
}>()

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'update:historyOpen', value: boolean): void
  (e: 'open', conversation: BotConversation): void
  (e: 'open-local'): void
  (e: 'load-more'): void
  (e: 'retry'): void
  (e: 'open-action-menu', conversation: BotConversation, event: MouseEvent | PointerEvent): void
  (e: 'update:menuQuery', value: string): void
  (e: 'select', item: SessionActionItem): void
}>()

const { t } = useI18n()
const title = computed(() => String(t(`bot.conversations.surfaces.${props.group.surface}`)))
const count = computed(() => (props.group.surface === 'local' ? props.group.conversations.length : props.group.total))
const historyCount = computed(() =>
  Math.max(props.group.history.length, props.group.total - props.group.conversations.length),
)
</script>

<template>
  <section :data-testid="`bot-surface-${group.surface}`" :aria-label="title">
    <SidebarListItem
      :data-keyboard-item="`bot-group:${botId}:${group.surface}`"
      :data-keyboard-label="title"
      data-keyboard-level="2"
      :data-keyboard-expanded="String(open)"
      density="compact"
      :actions-floating="false"
      class="gap-1.5"
      @click="emit('update:open', !open)"
    >
      <template #icon>
        <component :is="open ? RiArrowDownSLine : RiArrowRightSLine" class="h-3.5 w-3.5 text-muted-foreground" />
      </template>
      <span class="truncate typography-ui-label text-muted-foreground">{{ title }}</span>
      <template #meta>
        <span class="font-mono text-[11px] text-muted-foreground/70">{{ group.loaded ? count : '—' }}</span>
      </template>
    </SidebarListItem>

    <div v-if="open" class="pb-1 pl-3">
      <SidebarSectionSkeleton v-if="group.loading && !group.loaded" :rows="1" compact />
      <template v-else>
        <p v-if="group.error" role="alert" class="break-words px-2 py-2 text-xs text-destructive">
          {{ group.error }}
          <button class="underline" @click="emit('retry')">{{ t('common.retry') }}</button>
        </p>
        <ul v-if="group.conversations.length" class="space-y-0.5">
          <BotConversationRow
            v-for="conversation in group.conversations"
            :key="conversation.id"
            :bot-name="botName"
            :conversation="conversation"
            :selected-session-id="selectedSessionId"
            :ui-is-compact-layout="uiIsCompactLayout"
            :menu-open="menuOpenFor(conversation.id)"
            :menu-query="menuQuery"
            :menu-items="menuItems"
            :menu-anchor-el="menuAnchorEl"
            :set-menu-ref="setMenuRef"
            @open="(value) => emit('open', value)"
            @open-action-menu="(value, event) => emit('open-action-menu', value, event)"
            @update:menu-query="(value) => emit('update:menuQuery', value)"
            @select="(item) => emit('select', item)"
          />
        </ul>
        <SidebarListItem
          v-else-if="group.surface === 'local' && group.loaded"
          :data-keyboard-item="`bot-local-open:${botId}`"
          :data-keyboard-label="String(t('bot.conversations.openLocal'))"
          data-keyboard-level="3"
          data-testid="sidebar-bot-open-local"
          class="gap-2"
          @click="emit('open-local')"
        >
          <template #icon><RiChat3Line class="h-4 w-4 text-muted-foreground" /></template>
          <span class="typography-ui-label">{{ t('bot.conversations.openLocal') }}</span>
        </SidebarListItem>
        <p
          v-else-if="group.loaded && !group.error"
          class="px-2 py-2 text-xs text-muted-foreground"
          data-testid="bot-surface-empty"
        >
          {{ t(`bot.conversations.surfaceHints.${group.surface}`) }}
        </p>

        <section v-if="group.surface === 'local' && historyCount > 0" data-testid="bot-local-history">
          <SidebarListItem
            :data-keyboard-item="`bot-history:${botId}`"
            :data-keyboard-label="String(t('bot.conversations.history'))"
            data-keyboard-level="3"
            :data-keyboard-expanded="String(historyOpen)"
            density="compact"
            :actions-floating="false"
            class="gap-1.5"
            @click="emit('update:historyOpen', !historyOpen)"
          >
            <template #icon>
              <component
                :is="historyOpen ? RiArrowDownSLine : RiArrowRightSLine"
                class="h-3.5 w-3.5 text-muted-foreground"
              />
            </template>
            <span class="truncate typography-ui-label text-muted-foreground">{{ t('bot.conversations.history') }}</span>
            <template #meta
              ><span class="font-mono text-[11px] text-muted-foreground/70">{{ historyCount }}</span></template
            >
          </SidebarListItem>
          <div v-if="historyOpen" class="pl-3">
            <ul class="space-y-0.5">
              <BotConversationRow
                v-for="conversation in group.history"
                :key="conversation.id"
                :bot-name="botName"
                :conversation="conversation"
                :selected-session-id="selectedSessionId"
                :ui-is-compact-layout="uiIsCompactLayout"
                :menu-open="menuOpenFor(conversation.id)"
                :menu-query="menuQuery"
                :menu-items="menuItems"
                :menu-anchor-el="menuAnchorEl"
                :set-menu-ref="setMenuRef"
                @open="(value) => emit('open', value)"
                @open-action-menu="(value, event) => emit('open-action-menu', value, event)"
                @update:menu-query="(value) => emit('update:menuQuery', value)"
                @select="(item) => emit('select', item)"
              />
            </ul>
            <SidebarTextButton
              v-if="group.hasMore"
              class="px-2 py-1.5 text-xs"
              :disabled="group.loading"
              @click="emit('load-more')"
            >
              {{ t(group.loading ? 'common.loading' : 'common.loadMore') }}
            </SidebarTextButton>
          </div>
        </section>
        <SidebarTextButton
          v-if="group.surface !== 'local' && group.hasMore"
          class="px-2 py-1.5 text-xs"
          :disabled="group.loading"
          @click="emit('load-more')"
        >
          {{ t(group.loading ? 'common.loading' : 'common.loadMore') }}
        </SidebarTextButton>
      </template>
    </div>
  </section>
</template>
