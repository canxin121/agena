<script setup lang="ts">
import { computed, defineAsyncComponent, inject, provide, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftLine, RiArrowRightSLine, RiExternalLinkLine, RiGitBranchLine, RiRefreshLine } from '@remixicon/vue'
import type { Session } from '@/types/chat'
import { useChatStore } from '@/stores/chat'
import { listSessions } from '@/stores/chat/api'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { openBrowserKey } from '@/features/browser/api'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SessionChip from './SessionChip.vue'

const AgentConversationPreview = defineAsyncComponent(() => import('./AgentConversationPreview.vue'))
const props = defineProps<{ sessionId: string; busy: boolean; childCount?: number }>()
const { t } = useI18n()
const chat = useChatStore()
const open = ref(false)
const selectedId = ref<string>()
const title = computed(() => t('chat.operations.childSessions'))
const children = useVisibleResource<Session[]>({
  key: computed(() => props.sessionId),
  minInterval: 1000,
  interval: (): number =>
    props.busy || children.data.value?.some((child) => child.agent_status === 'running') ? 4000 : 30000,
  async load(parentId, signal) {
    const rows = new Map<string, Session>()
    const cursors = new Set<string>()
    let cursor: string | undefined
    do {
      const page = await listSessions({ parentId, cursor, limit: 100, excludeSubagents: false, force: true, signal })
      for (const child of page.sessions) {
        if (String(child.parent_id) === parentId) rows.set(child.id, child)
      }
      if (!page.hasMore) break
      if (!page.nextCursor || cursors.has(page.nextCursor)) throw new Error(t('chat.errors.historyPaginationStalled'))
      cursors.add(page.nextCursor)
      cursor = page.nextCursor
    } while (cursor)
    return [...rows.values()]
  },
})
const selected = computed(() => children.data.value?.find((child) => child.id === selectedId.value))
const count = computed(() => children.data.value?.length ?? props.childCount)
const label = computed(() => (count.value ? `${title.value} · ${count.value}` : title.value))
const statusKeys: Record<string, string> = {
  created: 'chat.partStatus.pending',
  running: 'chat.partStatus.running',
  completed: 'chat.partStatus.completed',
  failed: 'chat.partStatus.failed',
  cancelled: 'chat.partStatus.cancelled',
  timed_out: 'chat.operations.status.timed_out',
  interrupted: 'chat.messageItem.interrupted',
}
function name(child: Session) {
  return child.agent_name || child.title || `#${child.id}`
}
function status(child: Session) {
  const key = statusKeys[String(child.agent_status)]
  return key ? t(key) : ''
}
const openBrowser = inject(openBrowserKey, undefined)
if (openBrowser)
  provide(openBrowserKey, (sid, page) => {
    open.value = false
    openBrowser(sid, page)
  })
function enter(child: Session) {
  open.value = false
  void chat.selectSession(child.id)
}
watch([() => props.busy, () => props.childCount], () => void children.refresh())
watch(open, (value) => {
  if (value) void children.refresh()
  else selectedId.value = undefined
})
</script>

<template>
  <SessionChip
    v-if="open || count || children.error.value"
    data-children-chip
    :title="selected ? name(selected) : title"
    :label="label"
    :hint="label"
    :open="open"
    :busy="children.loading.value && !children.data.value"
    @update:open="open = $event"
  >
    <template #icon>
      <RiGitBranchLine class="h-3 w-3 shrink-0" aria-hidden="true" />
    </template>
    <template #actions>
      <template v-if="selected">
        <Button size="sm" variant="ghost" @click="selectedId = undefined">
          <RiArrowLeftLine aria-hidden="true" />{{ t('common.back') }}
        </Button>
        <Button size="sm" variant="ghost" @click="enter(selected)">
          <RiExternalLinkLine aria-hidden="true" />{{ t('common.open') }}
        </Button>
      </template>
      <IconButton
        v-else
        class="h-7 w-7"
        :tooltip="t('common.refresh')"
        :aria-label="t('common.refresh')"
        :disabled="children.loading.value"
        @click="children.refresh()"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <AgentConversationPreview v-if="selected" :key="selected.id" :session-id="selected.id" />
    <template v-else>
      <p v-if="children.error.value" role="alert" class="mb-1 break-words text-xs text-destructive">
        {{ children.error.value }}
      </p>
      <p v-else-if="!children.data.value?.length" class="text-xs text-muted-foreground">
        {{ t(children.data.value ? 'chat.toasts.noChildSessions' : 'common.loading') }}
      </p>
      <ul v-if="children.data.value?.length" class="space-y-0.5">
        <li v-for="child in children.data.value" :key="child.id">
          <button
            type="button"
            data-keyboard-option
            class="flex min-h-8 w-full min-w-0 items-center gap-2 rounded px-2 text-start text-xs hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
            :title="name(child)"
            @click="selectedId = child.id"
          >
            <span class="min-w-0 flex-1 truncate font-medium" dir="auto">{{ name(child) }}</span>
            <span v-if="status(child)" class="shrink-0 text-muted-foreground">{{ status(child) }}</span>
            <RiArrowRightSLine class="h-4 w-4 shrink-0 text-muted-foreground rtl:rotate-180" aria-hidden="true" />
          </button>
        </li>
      </ul>
    </template>
  </SessionChip>
</template>
