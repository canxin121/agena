<script setup lang="ts">
import { computed, ref, type ComponentPublicInstance } from 'vue'
import { RiArchiveLine, RiEditLine, RiRefreshLine, RiRobot2Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import IconButton from '@/components/ui/IconButton.vue'
import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import { formatTime } from '@/features/sessions/model/labels'
import type { BotProfile } from '@/lib/bots'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'

const props = withDefaults(
  defineProps<{
    profile: BotProfile
    selected?: boolean
    uiIsCompactLayout: boolean
    busy?: boolean
    menuOpen?: boolean
  }>(),
  {
    selected: false,
    busy: false,
    menuOpen: false,
  },
)

const emit = defineEmits<{
  (e: 'open'): void
  (e: 'update:menuOpen', v: boolean): void
  (e: 'action', action: 'edit' | 'rename' | 'refresh' | 'archive'): void
}>()

const { t } = useI18n()

const menuAnchorEl = ref<HTMLElement | null>(null)
const menuQuery = ref('')

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

const actionItems = computed<SessionActionItem[]>(() => [
  { id: 'edit', label: String(t('bot.profiles.edit')), icon: RiEditLine },
  { id: 'rename', label: String(t('bot.profiles.rename')), icon: RiEditLine },
  { id: 'refresh', label: String(t('common.refresh')), icon: RiRefreshLine },
  { id: 'archive', label: String(t('bot.profiles.archive')), icon: RiArchiveLine },
])

const visibleGroups = computed<OptionMenuGroup[]>(() => {
  const q = menuQuery.value.trim().toLowerCase()
  const items = q
    ? actionItems.value.filter((item) => item.label.toLowerCase().includes(q) || item.id.includes(q))
    : actionItems.value
  return [{ id: 'bot-actions', items: items as OptionMenuItem[] }]
})

function setMenuRef(el: Element | ComponentPublicInstance | null) {
  menuAnchorEl.value = el instanceof HTMLElement ? el : el?.$el instanceof HTMLElement ? el.$el : null
}

function openMenu(event: MouseEvent) {
  menuAnchorEl.value = event.currentTarget instanceof HTMLElement ? event.currentTarget : null
  menuQuery.value = ''
  emit('update:menuOpen', true)
}

function runAction(action: string) {
  emit('update:menuOpen', false)
  if (action === 'edit' || action === 'rename' || action === 'refresh' || action === 'archive') {
    emit('action', action)
  }
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
          @trigger="openMenu"
        />
        <template v-if="!uiIsCompactLayout">
          <IconButton
            data-testid="sidebar-bot-settings"
            size="xs"
            class="text-muted-foreground hover:text-foreground hover:dark:bg-accent/40 hover:bg-primary/6"
            :disabled="busy"
            :title="String(t('bot.profiles.edit'))"
            :aria-label="String(t('bot.profiles.edit'))"
            @click.stop="emit('action', 'edit')"
          >
            <RiEditLine class="h-4 w-4" />
          </IconButton>
        </template>
      </template>
    </SidebarListItem>

    <OptionMenu
      :ref="setMenuRef"
      :open="menuOpen"
      :query="menuQuery"
      :groups="visibleGroups"
      :title="menuLabel"
      :searchable="false"
      :is-compact-touch="false"
      :desktop-fixed="true"
      :desktop-anchor-el="menuAnchorEl"
      desktop-placement="bottom-end"
      desktop-class="w-56"
      filter-mode="external"
      @update:open="(v) => emit('update:menuOpen', v)"
      @select="(item) => runAction(item.id)"
    />
  </div>
</template>
