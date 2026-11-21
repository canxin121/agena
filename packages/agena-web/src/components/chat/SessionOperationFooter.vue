<script setup lang="ts">
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'
import { apiJson } from '@/lib/api'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import {
  composerProcesses,
  processCollection,
  type ProcessKind,
  type SessionProcess,
} from '@/pages/chat/sessionProcesses'
import { listSessions } from '@/stores/chat/api'
import type { MessageEntry, Session } from '@/types/chat'
import { readBrowserState } from '@/features/browser/api'
import { useBrowserCopy } from '@/features/browser/copy'
import BrowserTabChip from '@/features/browser/BrowserTabChip.vue'
import SessionAgentChip from './SessionAgentChip.vue'
import SessionProcessChip from './SessionProcessChip.vue'

const props = defineProps<{
  sessionId: string
  messages: MessageEntry[]
  busy: boolean
  childCount?: number
  browserOpen: boolean
  browserRequested: boolean
}>()
const emit = defineEmits<{ browser: [pageId?: string] }>()
const { t } = useI18n()
const browserText = useBrowserCopy()
const session = computed(() => props.sessionId)

// Read native collections on entry, including work whose launch Part is outside the transcript window.
function processList(kind: ProcessKind) {
  const reader = useVisibleResource<SessionProcess[]>({
    key: session,
    minInterval: 1000,
    interval: (): number => (props.busy || reader.data.value?.some((row) => row.status === 'running') ? 3000 : 30000),
    load: (id, signal) => apiJson<SessionProcess[]>(processCollection(id, kind), { signal, cache: 'no-store' }),
  })
  return reader
}
const shells = processList('shell')
const monitors = processList('monitor')
const processes = computed(() =>
  composerProcesses(props.sessionId, props.messages, shells.data.value ?? [], monitors.data.value ?? []),
)

const agents = useVisibleResource<Session[]>({
  key: session,
  minInterval: 1000,
  interval: (): number =>
    props.busy || agents.data.value?.some((agent) => agent.agent_status === 'running') ? 4000 : 30000,
  async load(parentId, signal) {
    const children = new Map<string, Session>()
    const cursors = new Set<string>()
    let cursor: string | undefined
    do {
      const page = await listSessions({ parentId, cursor, limit: 100, excludeSubagents: false, force: true, signal })
      for (const child of page.sessions) {
        if (String(child.parent_id) === parentId && (child.relation_kind === 'subagent' || child.is_subagent === true))
          children.set(child.id, child)
      }
      if (!page.hasMore) break
      if (!page.nextCursor || cursors.has(page.nextCursor)) throw new Error(t('chat.errors.historyPaginationStalled'))
      cursors.add(page.nextCursor)
      cursor = page.nextCursor
    } while (cursor)
    return [...children.values()]
  },
})
const browser = useVisibleResource({
  key: session,
  minInterval: 800,
  interval: (): number => (props.busy || browser.data.value?.tabs.length || props.browserRequested ? 2500 : 30000),
  load: readBrowserState,
})
const tabs = computed(() => browser.data.value?.tabs ?? [])
const helpPage = computed(() => browser.data.value?.help?.page_id)
const reads = [
  { label: () => browserText('browser'), reader: browser },
  { label: () => t('chat.operations.agent'), reader: agents },
  { label: () => t('chat.operations.process'), reader: shells },
  { label: () => t('chat.operations.subscription'), reader: monitors },
]
const errors = computed(() => reads.filter(({ reader }) => reader.error.value))
watch(
  () => props.busy,
  () => {
    for (const { reader } of reads) void reader.refresh()
  },
)
watch(
  () => props.childCount,
  () => void agents.refresh(),
)
watch([() => props.browserOpen, () => props.browserRequested], () => void browser.refresh())
</script>

<template>
  <div
    v-if="tabs.length || processes.length || agents.data.value?.length || browserRequested || errors.length"
    class="shrink-0"
    data-composer-resources
  >
    <div class="chat-column">
      <div
        class="resource-strip flex min-w-0 items-center gap-1.5 overflow-x-auto overscroll-x-contain pb-1 pt-0.5"
        data-oc-keyboard-tap="blur"
      >
        <BrowserTabChip
          v-for="tab in tabs"
          :key="tab.page_id"
          :session-id="sessionId"
          :tab="tab"
          :human="Boolean(browser.data.value?.human_control)"
          :requested="browserRequested && tab.page_id === helpPage"
          :viewer-open="browserOpen"
          @open="emit('browser', $event)"
        />
        <button
          v-if="browserRequested && !tabs.some((tab) => tab.page_id === helpPage)"
          type="button"
          class="composer-resource text-primary"
          aria-haspopup="dialog"
          @click="emit('browser', helpPage)"
        >
          {{ browserText('requested') }}
        </button>
        <SessionAgentChip v-for="agent in agents.data.value" :key="agent.id" :agent="agent" />
        <SessionProcessChip
          v-for="entry in processes"
          :key="entry.key"
          :session-id="sessionId"
          :entry="entry"
          @changed="(entry.kind === 'shell' ? shells : monitors).refresh()"
        />
        <button
          v-for="item in errors"
          :key="item.label()"
          type="button"
          class="composer-resource"
          :title="item.reader.error.value"
          @click="item.reader.refresh()"
        >
          <RiRefreshLine class="h-4 w-4" aria-hidden="true" />{{ item.label() }} · {{ t('common.retry') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.resource-strip {
  scrollbar-width: thin;
}
.resource-strip :deep(.composer-resource) {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 0.5rem;
  min-height: 2.75rem;
  max-width: min(17rem, 75vw);
  padding: 0.2rem 0.6rem;
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  background: var(--color-background);
  font-size: 0.75rem;
  line-height: 1rem;
  cursor: pointer;
}
.resource-strip :deep(.composer-resource:hover),
.resource-strip :deep(.composer-resource[aria-expanded='true']) {
  background: var(--color-muted);
}
.resource-strip :deep(.composer-resource:focus-visible) {
  outline: 2px solid var(--color-ring);
  outline-offset: -2px;
}
.resource-strip :deep(.resource-icon) {
  width: 1.1rem;
  height: 1.1rem;
  flex: none;
  color: var(--color-muted-foreground);
}
.resource-strip :deep(.resource-name),
.resource-strip :deep(.resource-caption) {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.resource-strip :deep(.resource-name) {
  color: var(--color-foreground);
  font-weight: 500;
}
.resource-strip :deep(.resource-caption) {
  color: var(--color-muted-foreground);
  font-size: 0.67rem;
}
</style>
