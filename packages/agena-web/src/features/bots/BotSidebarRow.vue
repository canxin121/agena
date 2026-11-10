<script setup lang="ts">
// A Bot row renders through the same sidebar primitives as a project session
// row: the shared action-item builder, the shared anchored menu component and
// the shared menu state machine in `useSessionActionMenu`. Only the handler for
// the conversation-level actions differs, because a Bot's conversation is
// already the pane the row opens.
import { computed, type ComponentPublicInstance } from 'vue'
import { RiRobot2Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import { formatTime } from '@/features/sessions/model/labels'
import type { BotProfile } from '@/lib/bots'
import SidebarSessionActionMenu from '@/layout/chatSidebar/components/SidebarSessionActionMenu.vue'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import type { MenuRefEl } from '@/layout/chatSidebar/sessionMenu.types'

const props = withDefaults(
  defineProps<{
    profile: BotProfile
    selected?: boolean
    uiIsCompactLayout: boolean
    busy?: boolean
    menuOpen?: boolean
    menuQuery?: string
    menuItems?: SessionActionItem[]
    menuAnchorEl?: HTMLElement | null
    setMenuRef?: (el: MenuRefEl) => void
    menuPlacement?: 'top-start' | 'top-end' | 'bottom-start' | 'bottom-end'
  }>(),
  {
    selected: false,
    busy: false,
    menuOpen: false,
    menuQuery: '',
    menuItems: () => [],
    menuAnchorEl: null,
    menuPlacement: 'bottom-end',
  },
)

const emit = defineEmits<{
  (e: 'open'): void
  (e: 'open-action-menu', event: MouseEvent | PointerEvent): void
  (e: 'update:menuQuery', v: string): void
  (e: 'select', item: SessionActionItem): void
}>()

const { t } = useI18n()

const initials = computed(() => Array.from(props.profile.name.trim()).slice(0, 2).join('').toLocaleUpperCase())
const subtitle = computed(() => {
  const instructions = String(props.profile.persona.instructions || '')
    .trim()
    .split('\n')[0]
    .trim()
  if (!instructions) return String(t('bot.profiles.instructionsNotSet'))
  return instructions.length > 100 ? `${instructions.slice(0, 100)}…` : instructions
})
const updatedAt = computed(() => {
  const ms = Number(props.profile.updated_at_ms || 0)
  return Number.isFinite(ms) ? ms : 0
})
const actionsAlwaysVisible = computed(() => props.uiIsCompactLayout)
const menuLabel = computed(() => String(t('bot.profiles.actions', { name: props.profile.name })))

function setMenuRef(el: Element | ComponentPublicInstance | null) {
  props.setMenuRef?.(el)
}

function onMenuSelect(item: SessionActionItem) {
  emit('select', item)
}

function handleClick() {
  if (props.menuOpen) return
  emit('open')
}
</script>

<template>
  <div class="group relative">
    <SidebarListItem
      :data-keyboard-item="`bot:${profile.id}`"
      :data-keyboard-label="profile.name"
      data-keyboard-level="2"
      :data-keyboard-selected="String(selected)"
      :active="selected"
      :actions-always-visible="actionsAlwaysVisible"
      class="relative gap-2"
      @click="handleClick"
    >
      <template #icon>
        <span
          v-if="initials"
          aria-hidden="true"
          class="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-primary/10 text-[10px] font-semibold text-primary"
          >{{ initials }}</span
        >
        <RiRobot2Line v-else class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      </template>

      <div class="flex w-full min-w-0 items-center gap-2">
        <div class="flex min-w-0 flex-1 flex-col justify-center">
          <span dir="auto" class="min-w-0 truncate text-left typography-ui-label">{{ profile.name }}</span>
          <span class="w-full truncate text-left text-[10px] text-muted-foreground/70">{{ subtitle }}</span>
        </div>
        <span class="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground/60">
          {{ formatTime(updatedAt) }}
        </span>
      </div>

      <template #actions>
        <ListItemOverflowActionButton
          data-testid="sidebar-bot-actions"
          :mobile="uiIsCompactLayout"
          :label="menuLabel"
          @trigger="(event) => emit('open-action-menu', event)"
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
      :desktop-placement="menuPlacement"
      @update:query="(v) => emit('update:menuQuery', v)"
      @select="onMenuSelect"
    />
  </div>
</template>
