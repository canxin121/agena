<script setup lang="ts">
import { computed, nextTick, ref, watch, type ComponentPublicInstance } from 'vue'
import { RiCheckLine, RiCloseLine, RiLoader4Line, RiMessageLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import IconButton from '@/components/ui/IconButton.vue'
import ListItemOverflowActionButton from '@/components/ui/ListItemOverflowActionButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import SidebarSessionActionMenu from '@/layout/chatSidebar/components/SidebarSessionActionMenu.vue'
import { sessionLabel } from '@/features/sessions/model/labels'
import { getIntlLocale } from '@/i18n/intl'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import type { Session } from '@/types/chat'

const { t } = useI18n()

const props = withDefaults(
  defineProps<{
    sessionId: string
    session?: Session | null

    uiIsCompactLayout: boolean
    selected?: boolean

    statusLabel?: string
    statusDotClass?: string
    showTime?: boolean

    renaming?: boolean
    renameDraft?: string
    renameBusy?: boolean

    sessionActionMenuOpen?: boolean
    sessionActionMenuAnchorEl?: HTMLElement | null
    sessionActionMenuQuery?: string
    filteredSessionActionItems?: SessionActionItem[]
    setSessionActionMenuRef?: (el: Element | ComponentPublicInstance | null) => void
    runSessionActionMenu?: (item: SessionActionItem) => Promise<void>
    menuPlacement?: 'top-start' | 'top-end' | 'bottom-start' | 'bottom-end'
  }>(),
  {
    session: null,
    selected: false,
    statusLabel: '',
    statusDotClass: '',
    showTime: true,
    renaming: false,
    renameDraft: '',
    renameBusy: false,
    sessionActionMenuOpen: false,
    sessionActionMenuAnchorEl: null,
    sessionActionMenuQuery: '',
    filteredSessionActionItems: () => [],
    menuPlacement: 'bottom-start',
  },
)

const emit = defineEmits<{
  (e: 'open'): void
  (e: 'open-actions'): void
  (e: 'open-action-menu', event: MouseEvent): void
  (e: 'update:renameDraft', v: string): void
  (e: 'rename-save'): void
  (e: 'rename-cancel'): void
  (e: 'update:sessionActionMenuQuery', v: string): void
}>()

const hasSession = computed(() => Boolean(props.session))
const rowRootEl = ref<HTMLElement | null>(null)
const renameInputEl = ref<HTMLInputElement | null>(null)

const isInlineRename = computed(() => props.renaming && !props.uiIsCompactLayout)
const renameDraftText = computed(() => String(props.renameDraft || ''))
const canSaveRename = computed(() => !props.renameBusy && renameDraftText.value.trim().length > 0)

const titleText = computed(() => {
  if (!props.session) return props.sessionId
  return sessionLabel(props.session) || String(t('hub.untitled'))
})

const statusLabelText = computed(() => {
  const next = String(props.statusLabel || '').trim()
  return next.length > 0 ? next : String(t('chat.sidebar.sessionRow.status.idle'))
})

const messageCount = computed(() => Math.max(0, Number(props.session?.message_count) || 0))

function parseTs(iso?: string | null): number {
  if (!iso) return 0
  const ms = Date.parse(iso)
  return Number.isFinite(ms) ? ms : 0
}

const updatedAtMs = computed(() => {
  if (!props.session) return 0
  return (
    parseTs(props.session.last_message_at) ||
    parseTs(props.session.updated_at) ||
    parseTs(props.session.created_at) ||
    0
  )
})

const timeText = computed(() => formatRelativeTime(updatedAtMs.value, Date.now()))

function formatRelativeTime(ms: number, now: number): string {
  if (!ms || !Number.isFinite(ms)) return ''
  const diffSec = Math.round((ms - now) / 1000)
  const abs = Math.abs(diffSec)
  let rtf: Intl.RelativeTimeFormat
  try {
    rtf = new Intl.RelativeTimeFormat(getIntlLocale(), { numeric: 'auto' })
  } catch {
    return new Date(ms).toLocaleString()
  }
  if (abs < 60) return rtf.format(diffSec, 'second')
  if (abs < 3600) return rtf.format(Math.round(diffSec / 60), 'minute')
  if (abs < 86400) return rtf.format(Math.round(diffSec / 3600), 'hour')
  if (abs < 86400 * 30) return rtf.format(Math.round(diffSec / 86400), 'day')
  if (abs < 86400 * 365) return rtf.format(Math.round(diffSec / (86400 * 30)), 'month')
  return rtf.format(Math.round(diffSec / (86400 * 365)), 'year')
}

const statusIndicator = computed<{ label: string; dotClass: string; pulse: boolean } | null>(() => {
  const dotClass = String(props.statusDotClass || '').trim()
  if (dotClass) {
    return {
      label: statusLabelText.value,
      dotClass,
      pulse: false,
    }
  }
  const state = String(props.session?.state || '').trim()
  switch (state) {
    case 'running':
      return {
        label: String(t('chat.sidebar.sessionRow.status.running')),
        dotClass: 'bg-sky-500',
        pulse: true,
      }
    case 'awaiting_user':
      return {
        label: String(t('chat.sidebar.sessionRow.status.needsReply')),
        dotClass: 'bg-amber-500',
        pulse: true,
      }
    case 'interrupted':
      return {
        label: 'Interrupted',
        dotClass: 'bg-amber-500',
        pulse: false,
      }
    case 'failed':
      return {
        label: 'Failed',
        dotClass: 'bg-destructive',
        pulse: false,
      }
    case 'creating':
      return {
        label: 'Creating',
        dotClass: 'bg-muted-foreground',
        pulse: true,
      }
    default:
      return null
  }
})

const canShowActions = computed(() => hasSession.value)
const actionsAlwaysVisible = computed(() => isInlineRename.value || (props.uiIsCompactLayout && canShowActions.value))

const shouldRenderSessionActionMenu = computed(() => {
  if (props.uiIsCompactLayout) return false
  if (isInlineRename.value) return false
  if (!props.sessionActionMenuOpen || !hasSession.value) return false
  const anchor = props.sessionActionMenuAnchorEl
  if (!anchor) return true
  return Boolean(rowRootEl.value?.contains(anchor))
})

watch(isInlineRename, (active) => {
  if (!active) return
  void nextTick(() => {
    const input = renameInputEl.value
    if (!input) return
    input.focus()
    input.select()
  })
})

function onRenameInput(event: Event) {
  const target = event.target as HTMLInputElement | null
  emit('update:renameDraft', target?.value || '')
}

function onActionMenuSelect(item: SessionActionItem) {
  if (!props.runSessionActionMenu) return
  void props.runSessionActionMenu(item)
}

function setMenuRef(el: Element | ComponentPublicInstance | null) {
  if (!props.setSessionActionMenuRef) return
  props.setSessionActionMenuRef(el)
}

function handleMobileOpenActionsClick() {
  emit('open-actions')
}

function handleDesktopOpenActionMenu(event: MouseEvent) {
  emit('open-action-menu', event)
}

function handleRowClick() {
  if (isInlineRename.value) return
  emit('open')
}
</script>

<template>
  <div ref="rowRootEl" class="group relative">
    <SidebarListItem
      :active="selected"
      :as="isInlineRename ? 'div' : 'button'"
      :actions-always-visible="actionsAlwaysVisible"
      class="gap-2 relative"
      @click="handleRowClick"
    >
      <template #icon>
        <div class="flex items-center gap-1.5 min-w-0">
          <span
            v-if="statusIndicator"
            class="inline-flex h-1.5 w-1.5 rounded-full flex-shrink-0"
            :class="[statusIndicator.dotClass, statusIndicator.pulse ? 'animate-pulse' : '']"
            :title="statusIndicator.label"
            :aria-label="statusIndicator.label"
          />
        </div>
      </template>

      <div class="flex w-full items-center min-w-0 gap-2">
        <template v-if="!isInlineRename">
          <div v-if="hasSession" class="flex-1 min-w-0 flex flex-col justify-center">
            <span class="truncate typography-ui-label w-full text-left">{{ titleText }}</span>
            <span
              v-if="showTime && timeText"
              class="truncate text-[10px] text-muted-foreground/70 w-full text-left"
              >{{ timeText }}</span
            >
          </div>

          <div v-else class="flex-1 min-w-0 flex flex-col justify-center">
            <div
              class="h-3 w-36 rounded bg-muted/30 animate-pulse"
              :aria-label="String(t('chat.sidebar.sessionRow.loading.session'))"
            />
          </div>

          <span
            v-if="messageCount > 0"
            class="ml-auto inline-flex items-center gap-1 font-mono text-[10px] text-muted-foreground/60 flex-shrink-0"
            :title="String(messageCount)"
          >
            <RiMessageLine class="h-3 w-3" />
            {{ messageCount }}
          </span>
        </template>

        <template v-else>
          <input
            ref="renameInputEl"
            type="text"
            :value="renameDraftText"
            class="h-7 min-w-0 flex-1 rounded-md border border-input bg-background/95 px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :placeholder="String(t('chat.sidebar.sessionRow.placeholders.sessionTitle'))"
            @click.stop
            @input="onRenameInput"
            @keydown.enter.prevent.stop="emit('rename-save')"
            @keydown.esc.prevent.stop="emit('rename-cancel')"
          />
        </template>
      </div>

      <template #actions>
        <template v-if="isInlineRename">
          <IconButton
            size="xs"
            class="text-muted-foreground hover:text-foreground hover:bg-primary/6"
            :title="String(t('chat.sidebar.sessionRow.rename.cancel'))"
            :aria-label="String(t('chat.sidebar.sessionRow.rename.cancel'))"
            :disabled="renameBusy"
            @click.stop="emit('rename-cancel')"
          >
            <RiCloseLine class="h-3.5 w-3.5" />
          </IconButton>
          <IconButton
            size="xs"
            class="text-primary hover:bg-primary/12"
            :title="
              String(t(renameBusy ? 'chat.sidebar.sessionRow.rename.saving' : 'chat.sidebar.sessionRow.rename.save'))
            "
            :aria-label="
              String(t(renameBusy ? 'chat.sidebar.sessionRow.rename.saving' : 'chat.sidebar.sessionRow.rename.save'))
            "
            :disabled="!canSaveRename"
            @click.stop="emit('rename-save')"
          >
            <RiLoader4Line v-if="renameBusy" class="h-3.5 w-3.5 animate-spin" />
            <RiCheckLine v-else class="h-3.5 w-3.5" />
          </IconButton>
        </template>

        <template v-else-if="canShowActions">
          <ListItemOverflowActionButton
            :mobile="uiIsCompactLayout"
            :label="String(t('chat.sidebar.sessionActions.menuTitle'))"
            @trigger="uiIsCompactLayout ? handleMobileOpenActionsClick() : handleDesktopOpenActionMenu($event)"
          />
        </template>
      </template>
    </SidebarListItem>

    <SidebarSessionActionMenu
      v-if="shouldRenderSessionActionMenu"
      :open="true"
      :query="sessionActionMenuQuery"
      :items="filteredSessionActionItems"
      :set-menu-ref="setMenuRef"
      :anchor-el="sessionActionMenuAnchorEl"
      :desktop-placement="menuPlacement"
      @update:query="(v) => emit('update:sessionActionMenuQuery', v)"
      @select="onActionMenuSelect"
    />
  </div>
</template>
