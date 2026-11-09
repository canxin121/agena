<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { RiAddLine, RiMore2Fill, RiRefreshLine } from '@remixicon/vue'
import { searchBots } from '@/features/bots/profiles'
import { useBotsStore } from '@/stores/bots'
import { useUiStore } from '@/stores/ui'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
const bots = useBotsStore()
const ui = useUiStore()
const navigation = useWorkspaceNavigation()
const route = useRoute()
const { t } = useI18n()
const query = ref('')
const menu = ref('')
const section = ref<HTMLElement | null>(null)
const expanded = ref(true)
watch(query, () => {
  menu.value = ''
})
function outside(event: PointerEvent) {
  if (!section.value?.contains(event.target as Node)) menu.value = ''
}
onMounted(() => document.addEventListener('pointerdown', outside))
onBeforeUnmount(() => document.removeEventListener('pointerdown', outside))
function toggleMenu(id: string) {
  menu.value = menu.value === id ? '' : id
  if (menu.value) void nextTick(() => section.value?.querySelector<HTMLButtonElement>('[role=menuitem]')?.focus())
}
function closeMenu(event: KeyboardEvent) {
  const row = (event.currentTarget as HTMLElement).closest('li')
  menu.value = ''
  void nextTick(() => row?.querySelector<HTMLButtonElement>('[data-testid=sidebar-bot-actions]')?.focus())
}
const visible = computed(() => searchBots(bots.profiles, query.value))
function open(id: string, name: string) {
  ui.setSessionSwitcherOpen(false)
  void navigation.openWorkspaceLocation('bot', { query: { botId: id }, title: name, matchKeys: ['botId'] })
}
function menuKey(event: KeyboardEvent) {
  if (!['ArrowDown', 'ArrowUp'].includes(event.key)) return
  const buttons = Array.from(
    (event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('[role=menuitem]'),
  )
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
  event.preventDefault()
  buttons[(index + (event.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus({
    preventScroll: true,
  })
}
function openDefault() {
  ui.setSessionSwitcherOpen(false)
  void navigation.openMainTab('bot')
}
function toggleExpanded() {
  expanded.value = !expanded.value
  menu.value = ''
}
function dialog(kind: 'create' | 'edit' | 'rename' | 'archive', id?: string) {
  menu.value = ''
  bots.open(kind, id)
}
</script>
<template>
  <section ref="section" data-testid="sidebar-bots" class="shrink-0 border-b border-sidebar-border/60 px-3 pb-2 pt-3">
    <div class="flex items-center gap-1">
      <button data-testid="sidebar-bot" class="min-w-0 flex-1 text-start text-sm font-semibold" @click="openDefault">
        {{ t('bot.title') }} <span class="text-xs font-normal text-muted-foreground">{{ bots.profiles.length }}</span>
      </button>
      <button
        data-testid="sidebar-bot-refresh"
        type="button"
        class="rounded-md p-2 hover:bg-muted"
        :aria-label="t('common.refresh')"
        @click="bots.refresh(true)"
      >
        <RiRefreshLine class="h-4 w-4" />
      </button>
      <button
        data-testid="sidebar-bot-create"
        type="button"
        class="flex items-center gap-1 rounded-lg bg-primary/10 px-2 py-1.5 text-xs font-semibold text-primary hover:bg-primary/20"
        @click="dialog('create')"
      >
        <RiAddLine class="h-4 w-4" />{{ t('bot.profiles.create') }}
      </button>
    </div>
    <button
      type="button"
      class="mt-1 text-xs text-muted-foreground underline"
      :aria-expanded="expanded"
      @click="toggleExpanded"
    >
      {{ expanded ? t('codeBlock.collapse') : t('codeBlock.expand') }}
    </button>
    <div v-show="expanded" class="relative mt-2">
      <input
        v-model="query"
        @focus="menu = ''"
        data-testid="sidebar-bot-search"
        :aria-label="t('bot.profiles.search')"
        :placeholder="t('bot.profiles.search')"
        class="h-8 w-full rounded-md border border-input bg-background px-2 pe-8 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
      <button
        v-if="query"
        type="button"
        class="absolute end-1 top-0 h-8 px-2 text-muted-foreground"
        :aria-label="t('common.clear')"
        @click="query = ''"
      >
        ×
      </button>
    </div>
    <p
      v-if="expanded && bots.loading && !bots.profiles.length"
      role="status"
      class="py-2 text-xs text-muted-foreground"
    >
      {{ t('common.loading') }}
    </p>
    <div v-if="expanded && bots.error" role="alert" class="break-words py-2 text-xs text-destructive">
      {{ bots.error }} <button class="underline" @click="bots.refresh(true)">{{ t('common.retry') }}</button>
    </div>
    <p v-if="expanded && !bots.loading && !bots.error && !visible.length" class="py-2 text-xs text-muted-foreground">
      {{ query.trim() ? t('bot.profiles.noMatches') : t('bot.profiles.empty') }}
    </p>
    <ul
      v-show="expanded"
      class="mt-1 max-h-[min(12rem,25dvh)] space-y-0.5 overflow-y-auto overscroll-contain"
      :aria-label="t('bot.title')"
    >
      <li
        v-for="profile in visible"
        :key="profile.id"
        :data-sidebar-bot-id="profile.id"
        class="relative flex min-w-0 flex-wrap items-center rounded-lg"
        :class="route.path === '/bot' && route.query.botId === profile.id ? 'bg-primary/10' : 'hover:bg-muted/70'"
      >
        <button
          type="button"
          data-testid="sidebar-bot-open"
          class="flex min-h-11 min-w-0 flex-1 items-center gap-2 px-1.5 py-1 text-start focus-visible:ring-2 focus-visible:ring-ring"
          :title="profile.name"
          :aria-current="route.path === '/bot' && route.query.botId === profile.id ? 'page' : undefined"
          @click="open(profile.id, profile.name)"
        >
          <span
            aria-hidden="true"
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-xs font-semibold text-primary"
            >{{ Array.from(profile.name.trim()).slice(0, 2).join('').toLocaleUpperCase() }}</span
          >
          <span class="min-w-0 flex-1"
            ><span dir="auto" class="line-clamp-2 break-all text-xs font-semibold leading-tight">{{
              profile.name
            }}</span
            ><span class="block truncate text-[10px] text-muted-foreground">{{
              profile.persona.instructions || t('bot.profiles.instructionsNotSet')
            }}</span></span
          >
        </button>
        <div class="relative shrink-0" @keydown.esc.stop="closeMenu">
          <button
            type="button"
            data-testid="sidebar-bot-actions"
            class="h-10 w-9 rounded-md hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
            :aria-label="t('bot.profiles.actions', { name: profile.name })"
            aria-haspopup="menu"
            :aria-expanded="menu === profile.id"
            @click="toggleMenu(profile.id)"
          >
            <RiMore2Fill class="mx-auto h-4 w-4" />
          </button>
        </div>
        <div
          v-if="menu === profile.id"
          role="menu"
          class="w-full rounded-lg border bg-popover p-1 text-popover-foreground"
          @keydown.esc.stop="closeMenu"
          @keydown="menuKey"
        >
          <button
            role="menuitem"
            data-testid="sidebar-bot-edit"
            class="w-full rounded px-2 py-2 text-start text-xs hover:bg-muted"
            @click="dialog('edit', profile.id)"
          >
            {{ t('bot.profiles.edit') }}
          </button>
          <button
            role="menuitem"
            data-testid="sidebar-bot-rename"
            class="w-full rounded px-2 py-2 text-start text-xs hover:bg-muted"
            @click="dialog('rename', profile.id)"
          >
            {{ t('bot.profiles.rename') }}
          </button>
          <button
            role="menuitem"
            data-testid="sidebar-bot-archive"
            class="w-full rounded px-2 py-2 text-start text-xs text-destructive hover:bg-muted"
            @click="dialog('archive', profile.id)"
          >
            {{ t('bot.profiles.archive') }}
          </button>
        </div>
      </li>
    </ul>
  </section>
</template>
