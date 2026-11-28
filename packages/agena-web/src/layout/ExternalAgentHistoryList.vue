<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiArrowLeftSLine,
  RiArrowRightSLine,
  RiEyeLine,
  RiInboxArchiveLine,
  RiInboxUnarchiveLine,
  RiLoader4Line,
  RiRefreshLine,
} from '@remixicon/vue'

import AcpInstallGuideCard from '@/components/acp/AcpInstallGuideCard.vue'
import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import SegmentedButton from '@/components/ui/SegmentedButton.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import {
  listExternalAgents,
  listNativeAgentSessions,
  openNativeAgentSession,
  previewNativeAgentHistory,
  setNativeAgentArchived,
  useExternalAgentCatalog,
  watchNativeAgentSessions,
  type NativeArchiveFilter,
  type NativeHistoryPreview,
  type NativeIndexedSession,
  type NativeIndexCapabilities,
} from '@/features/sessions/api/nativeAgentSessions'
import { isMissingAcpCommandError } from '@/lib/acpInstallGuide'
import { useUiStore } from '@/stores/ui'
import { formatExternalAgentLabel, type ExternalAgent } from '@/types/chat'

const props = defineProps<{ workspace: number; agent: ExternalAgent; agentLabel?: string }>()
const emit = defineEmits<{
  (event: 'opened', session: number): void
}>()
const { locale, t } = useI18n()
const ui = useUiStore()
const catalog = useExternalAgentCatalog()
const agentEntry = computed(() => catalog.value.find((entry) => entry.id === props.agent))
const sessions = ref<NativeIndexedSession[]>([])
const cursor = ref<string | null>(null)
const capabilities = ref<NativeIndexCapabilities | null>(null)
const archived = ref<NativeArchiveFilter>('unarchived')
const searchQuery = ref('')
const loading = ref(false)
const mutation = ref<{ id: string; action: 'open' | 'archive' } | null>(null)
const loaded = ref(false)
const error = ref('')
const showInstallGuide = computed(
  () => agentEntry.value?.available === false || isMissingAcpCommandError(error.value),
)
const preview = ref<{
  session: NativeIndexedSession
  page: NativeHistoryPreview | null
  cursors: (number | null)[]
  error: string
} | null>(null)
const previewLoading = ref(false)
let read: AbortController | undefined
let watcher: AbortController | undefined
let actionRequest: AbortController | undefined
let previewRequest: AbortController | undefined
let refreshPending = false

const filteredSessions = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return sessions.value
  return sessions.value.filter((session) =>
    [session.title || '', session.native_id, session.model || ''].some((value) => value.toLowerCase().includes(q)),
  )
})

const showSearch = computed(() => sessions.value.length > 3 || searchQuery.value.trim().length > 0)

function refreshWhenIdle() {
  if (refreshPending && !loading.value && !mutation.value) {
    void load(false)
  }
}

function follow() {
  if (watcher || !capabilities.value?.watch) return
  const scope = new AbortController()
  watcher = scope
  const { workspace, agent } = props
  void (async () => {
    try {
      while (!scope.signal.aborted) {
        const change = await watchNativeAgentSessions(workspace, agent, scope.signal)
        if (scope.signal.aborted || !change.supported) break
        if (change.changed) {
          refreshPending = true
          refreshWhenIdle()
          await new Promise((resolve) => window.setTimeout(resolve, 300))
        }
      }
    } catch {
      // Manual refresh remains available if the agent stops its subscription.
    } finally {
      if (watcher === scope) watcher = undefined
    }
  })()
}

async function load(append: boolean) {
  read?.abort()
  const request = new AbortController()
  read = request
  loading.value = true
  refreshPending = false
  error.value = ''
  try {
    const page = await listNativeAgentSessions(
      props.workspace,
      props.agent,
      append ? cursor.value : null,
      archived.value,
      request.signal,
    )
    if (request.signal.aborted) return
    if (append) {
      const known = new Set(sessions.value.map((session) => session.native_id))
      sessions.value.push(...page.sessions.filter((session) => !known.has(session.native_id)))
    } else {
      sessions.value = page.sessions
    }
    cursor.value = page.next_cursor
    capabilities.value = page.capabilities
    loaded.value = true
    if (page.capabilities.watch) follow()
    else watcher?.abort()
  } catch (failure) {
    if (!request.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    if (read === request) {
      loading.value = false
      refreshWhenIdle()
    }
  }
}

async function changeSession(nativeId: string, action: 'open' | 'archive', nextArchived = false) {
  if (mutation.value) return
  const request = new AbortController()
  actionRequest = request
  mutation.value = { id: nativeId, action }
  error.value = ''
  try {
    if (action === 'open') {
      const id = await openNativeAgentSession(props.workspace, props.agent, nativeId, request.signal)
      if (!request.signal.aborted) emit('opened', id)
    } else {
      await setNativeAgentArchived(props.workspace, props.agent, nativeId, nextArchived)
      if (!request.signal.aborted) await load(false)
    }
  } catch (failure) {
    if (!request.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    mutation.value = null
    refreshWhenIdle()
  }
}

function showPreview(session: NativeIndexedSession) {
  preview.value = { session, page: null, cursors: [null], error: '' }
  void loadPreview([null])
}

function closePreview() {
  previewRequest?.abort()
  previewRequest = undefined
  previewLoading.value = false
  preview.value = null
}

async function openFromPreview() {
  const nativeId = preview.value?.session.native_id
  if (!nativeId) return
  closePreview()
  await changeSession(nativeId, 'open')
}

async function loadPreview(cursors: (number | null)[]) {
  const current = preview.value
  if (!current || previewLoading.value) return
  const request = new AbortController()
  previewRequest = request
  previewLoading.value = true
  current.error = ''
  try {
    const page = await previewNativeAgentHistory(
      props.workspace,
      props.agent,
      current.session.native_id,
      cursors.at(-1) ?? null,
      request.signal,
    )
    if (request.signal.aborted) return
    current.page = page
    current.cursors = cursors
  } catch (failure) {
    if (!request.signal.aborted) current.error = failure instanceof Error ? failure.message : String(failure)
  } finally {
    if (previewRequest === request) previewLoading.value = false
  }
}

function previewEarlier() {
  const current = preview.value
  const nextCursor = current?.page?.next_before_turn
  if (current && nextCursor != null) void loadPreview([...current.cursors, nextCursor])
}

function setArchiveFilter(next: NativeArchiveFilter) {
  if (archived.value === next || mutation.value !== null) return
  archived.value = next
  sessions.value = []
  cursor.value = null
  loaded.value = false
  void load(false)
}

function canChangeArchive(session: NativeIndexedSession) {
  return !capabilities.value?.index || (session.state !== 'running' && session.state !== 'requires_action')
}

function fullDateLabel(value: string | null) {
  const timestamp = value ? Date.parse(value) : NaN
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString(locale.value) : ''
}

function relativeDateLabel(value: string | null) {
  const timestamp = value ? Date.parse(value) : NaN
  if (!Number.isFinite(timestamp)) return ''
  const diffSeconds = Math.round((timestamp - Date.now()) / 1000)
  const abs = Math.abs(diffSeconds)
  try {
    const rtf = new Intl.RelativeTimeFormat(locale.value, { numeric: 'auto', style: 'narrow' })
    if (abs < 60) return rtf.format(diffSeconds, 'second')
    if (abs < 3600) return rtf.format(Math.round(diffSeconds / 60), 'minute')
    if (abs < 86400) return rtf.format(Math.round(diffSeconds / 3600), 'hour')
    if (abs < 86400 * 30) return rtf.format(Math.round(diffSeconds / 86400), 'day')
  } catch {
    // Fallback to locale date below.
  }
  return new Date(timestamp).toLocaleDateString(locale.value)
}

function stateBadgeClass(state: string | null) {
  if (state === 'running') return 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300'
  if (state === 'requires_action') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300'
  return 'bg-muted/70 text-muted-foreground'
}

async function recheckAndLoad() {
  loading.value = true
  error.value = ''
  try {
    await listExternalAgents()
  } catch {
    // Still attempt history load below to surface the latest error.
  }
  if (agentEntry.value?.available === false) {
    loading.value = false
    return
  }
  await load(false)
}

watch(
  () => [props.workspace, props.agent],
  () => {
    closePreview()
    watcher?.abort()
    watcher = undefined
    capabilities.value = null
    archived.value = 'unarchived'
    searchQuery.value = ''
    sessions.value = []
    cursor.value = null
    loaded.value = false
    error.value = ''
    if (agentEntry.value?.available === false) {
      read?.abort()
      loading.value = false
      return
    }
    void load(false)
  },
  { immediate: true },
)

watch(
  () => agentEntry.value?.available,
  (next, previous) => {
    if (next === true && previous === false && !loaded.value && !loading.value) {
      void load(false)
    }
  },
)
onBeforeUnmount(() => {
  refreshPending = false
  read?.abort()
  watcher?.abort()
  actionRequest?.abort()
  previewRequest?.abort()
})
</script>

<template>
  <div class="space-y-2">
    <div class="flex items-center justify-between gap-2 px-1">
      <div class="flex min-w-0 items-center gap-1.5">
        <h3 class="truncate text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          {{ t('chat.nativeAgent.history', { agent: agentLabel || formatExternalAgentLabel(agent) }) }}
        </h3>
        <span
          v-if="loaded && sessions.length"
          class="shrink-0 rounded-full bg-muted/70 px-1.5 py-0.2 text-[10px] font-medium tabular-nums text-muted-foreground"
        >
          {{ sessions.length }}
        </span>
      </div>
      <IconButton
        v-if="loaded || error"
        size="sm"
        variant="ghost"
        class="h-6 w-6 shrink-0"
        :disabled="loading || mutation !== null"
        :tooltip="t(loaded ? 'common.refresh' : 'common.retry')"
        :aria-label="t(loaded ? 'common.refresh' : 'common.retry')"
        @click="load(false)"
      >
        <RiRefreshLine class="h-3.5 w-3.5" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <SegmentedControl
      v-if="capabilities?.archive"
      class="grid-cols-3"
      :aria-label="t('chat.nativeAgent.archiveFilter')"
    >
      <SegmentedButton :active="archived === 'unarchived'" @click="setArchiveFilter('unarchived')">
        {{ t('chat.nativeAgent.unarchived') }}
      </SegmentedButton>
      <SegmentedButton :active="archived === 'archived'" @click="setArchiveFilter('archived')">
        {{ t('chat.nativeAgent.archived') }}
      </SegmentedButton>
      <SegmentedButton :active="archived === 'all'" @click="setArchiveFilter('all')">
        {{ t('common.all') }}
      </SegmentedButton>
    </SegmentedControl>

    <SearchInput
      v-if="showSearch"
      v-model="searchQuery"
      :placeholder="t('chat.nativeAgent.searchHistory')"
      :input-aria-label="t('chat.nativeAgent.searchHistory')"
      :show-search-button="false"
    />

    <AcpInstallGuideCard
      v-if="showInstallGuide"
      compact
      :agent-id="agent"
      :agent-name="agentLabel || agentEntry?.name"
      :command="agentEntry?.command"
      :raw-error="error || undefined"
      :show-retry="true"
      :retrying="loading"
      @retry="recheckAndLoad"
    />
    <p
      v-else-if="error"
      role="alert"
      class="break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
    >
      {{ error }}
    </p>
    <div v-else-if="loading && !loaded" role="status" class="space-y-1.5 py-1">
      <div v-for="n in 3" :key="n" class="h-11 animate-pulse rounded-md bg-muted/40" />
    </div>
    <p v-else-if="loaded && !sessions.length && cursor === null" class="px-1 py-2 text-xs text-muted-foreground">
      {{ t('chat.nativeAgent.noHistory') }}
    </p>
    <p v-else-if="loaded && !filteredSessions.length" class="px-1 py-2 text-xs text-muted-foreground">
      {{ t('chat.nativeAgent.noHistoryMatch') }}
    </p>

    <ul v-if="filteredSessions.length" class="space-y-0.5">
      <li
        v-for="session in filteredSessions"
        :key="session.native_id"
        class="group relative flex items-center gap-1 rounded-md transition-colors hover:bg-sidebar-accent/70 focus-within:bg-sidebar-accent/70"
      >
        <button
          type="button"
          class="flex min-w-0 flex-1 flex-col gap-0.5 rounded-md px-2 py-1.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
          :disabled="mutation !== null"
          :title="session.title || session.native_id"
          @click="changeSession(session.native_id, 'open')"
        >
          <span class="flex items-center gap-1.5">
            <RiLoader4Line
              v-if="mutation?.id === session.native_id && mutation.action === 'open'"
              class="h-3.5 w-3.5 shrink-0 animate-spin text-primary"
            />
            <span class="min-w-0 flex-1 truncate text-xs font-medium text-foreground">
              {{ session.title || session.native_id }}
            </span>
          </span>
          <span class="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
            <span
              v-if="session.state === 'running' || session.state === 'requires_action'"
              class="inline-flex items-center gap-1 rounded px-1 py-0.2 text-[10px] font-medium"
              :class="stateBadgeClass(session.state)"
            >
              <span
                class="h-1.5 w-1.5 rounded-full"
                :class="session.state === 'running' ? 'animate-pulse bg-emerald-500' : 'bg-amber-500'"
                aria-hidden="true"
              />
              {{ t(`chat.externalAgentDetails.states.${session.state}`, session.state) }}
            </span>
            <span
              v-if="session.archived"
              class="rounded bg-muted/70 px-1 py-0.2 text-[10px] font-medium text-muted-foreground"
            >
              {{ t('chat.nativeAgent.archived') }}
            </span>
            <span v-if="session.model" class="max-w-28 truncate font-mono text-[10px] opacity-85">
              {{ session.model }}
            </span>
            <span
              v-if="relativeDateLabel(session.updated_at)"
              class="shrink-0 tabular-nums opacity-80"
              :title="fullDateLabel(session.updated_at)"
            >
              {{ relativeDateLabel(session.updated_at) }}
            </span>
          </span>
        </button>

        <div
          class="flex shrink-0 items-center gap-0.5 pr-1 transition-opacity"
          :class="
            ui.isTouchPointer || mutation?.id === session.native_id
              ? 'opacity-100'
              : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'
          "
        >
          <IconButton
            size="sm"
            variant="ghost"
            class="h-6 w-6"
            :disabled="mutation !== null"
            :tooltip="t('chat.nativeAgent.previewHistory')"
            :aria-label="t('chat.nativeAgent.previewHistory')"
            @click.stop="showPreview(session)"
          >
            <RiEyeLine class="h-3.5 w-3.5" />
          </IconButton>
          <IconButton
            v-if="capabilities?.archive && session.archived !== null"
            size="sm"
            variant="ghost"
            class="h-6 w-6"
            :disabled="mutation !== null || !canChangeArchive(session)"
            :tooltip="t(session.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive')"
            :aria-label="t(session.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive')"
            @click.stop="changeSession(session.native_id, 'archive', !session.archived)"
          >
            <RiLoader4Line
              v-if="mutation?.id === session.native_id && mutation.action === 'archive'"
              class="h-3.5 w-3.5 animate-spin"
            />
            <RiInboxUnarchiveLine v-else-if="session.archived" class="h-3.5 w-3.5" />
            <RiInboxArchiveLine v-else class="h-3.5 w-3.5" />
          </IconButton>
        </div>
      </li>
    </ul>

    <div v-if="cursor !== null" class="flex justify-center pt-1">
      <Button size="sm" variant="ghost" class="w-full text-xs" :disabled="loading || mutation !== null" @click="load(true)">
        <RiLoader4Line v-if="loading" class="mr-1.5 h-3.5 w-3.5 animate-spin" />
        {{ t('common.loadMore') }}
      </Button>
    </div>

    <Dialog
      :open="preview !== null"
      :title="preview?.session.title || preview?.session.native_id"
      :description="t('chat.nativeAgent.historyPreviewDescription')"
      max-width="max-w-2xl"
      @close="closePreview"
    >
      <div v-if="preview" class="space-y-3">
        <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span class="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[11px]">{{ preview.session.native_id }}</span>
          <span v-if="preview.session.model" class="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[11px]">
            {{ preview.session.model }}
          </span>
          <span v-if="fullDateLabel(preview.session.updated_at)">{{ fullDateLabel(preview.session.updated_at) }}</span>
        </div>

        <div v-if="previewLoading" role="status" class="space-y-2 py-2">
          <div v-for="n in 3" :key="n" class="h-16 animate-pulse rounded-lg bg-muted/40" />
        </div>
        <p
          v-else-if="preview.error"
          role="alert"
          class="break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
        >
          {{ preview.error }}
        </p>
        <template v-else-if="preview.page">
          <p
            v-if="preview.page.omitted_content"
            class="rounded-md border border-border/70 bg-muted/30 px-3 py-2 text-xs text-muted-foreground"
          >
            {{ t('chat.nativeAgent.historyPreviewOmitted') }}
          </p>
          <p v-if="!preview.page.turns.length" class="py-6 text-center text-sm text-muted-foreground">
            {{ t('chat.nativeAgent.historyPreviewEmpty') }}
          </p>
          <ol v-else class="space-y-3">
            <li
              v-for="(turn, index) in preview.page.turns"
              :key="index"
              class="space-y-2 rounded-lg border border-border/70 bg-card/40 p-3"
            >
              <section v-if="turn.user" class="rounded-md border border-primary/20 bg-primary/5 px-3 py-2">
                <h4 class="text-[11px] font-medium text-primary">{{ t('chat.externalAgentDetails.roles.user') }}</h4>
                <p class="mt-1 whitespace-pre-wrap break-words text-sm leading-relaxed">{{ turn.user }}</p>
              </section>
              <section v-if="turn.reply" class="px-1">
                <h4 class="text-[11px] font-medium text-muted-foreground">
                  {{ t('chat.externalAgentDetails.roles.assistant') }}
                </h4>
                <p class="mt-1 whitespace-pre-wrap break-words text-sm leading-relaxed">{{ turn.reply }}</p>
              </section>
            </li>
          </ol>
        </template>
      </div>
      <template #footer>
        <template v-if="preview">
          <div class="flex flex-1 flex-wrap items-center gap-2">
            <Button
              v-if="preview.page?.next_before_turn != null"
              size="sm"
              variant="outline"
              :disabled="previewLoading"
              @click="previewEarlier"
            >
              <RiArrowLeftSLine class="mr-1 h-4 w-4" />
              {{ t('chat.nativeAgent.historyPreviewEarlier') }}
            </Button>
            <Button
              v-if="preview.cursors.length > 1"
              size="sm"
              variant="outline"
              :disabled="previewLoading"
              @click="loadPreview(preview.cursors.slice(0, -1))"
            >
              {{ t('chat.nativeAgent.historyPreviewNewer') }}
              <RiArrowRightSLine class="ml-1 h-4 w-4" />
            </Button>
            <Button
              v-if="preview.error && !preview.page"
              size="sm"
              variant="outline"
              :disabled="previewLoading"
              @click="loadPreview(preview.cursors)"
            >
              {{ t('common.retry') }}
            </Button>
            <span v-if="preview.cursors.length > 1" class="text-xs tabular-nums text-muted-foreground">
              #{{ preview.cursors.length }}
            </span>
          </div>
          <Button size="sm" :disabled="mutation !== null" @click="openFromPreview">
            {{ t('chat.nativeAgent.openConversation') }}
          </Button>
        </template>
      </template>
    </Dialog>
  </div>
</template>
