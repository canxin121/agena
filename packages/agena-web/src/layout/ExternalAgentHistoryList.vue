<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import {
  listNativeAgentSessions,
  openNativeAgentSession,
  setNativeAgentArchived,
  watchNativeAgentSessions,
  type NativeArchiveFilter,
  type NativeIndexedSession,
  type NativeIndexCapabilities,
} from '@/features/sessions/api/nativeAgentSessions'
import { externalAgentLabels, type ExternalAgent } from '@/types/chat'

const props = defineProps<{ workspace: number; agent: ExternalAgent }>()
const emit = defineEmits<{
  (event: 'opened', session: number): void
}>()
const { t } = useI18n()
const sessions = ref<NativeIndexedSession[]>([])
const cursor = ref<string | null>(null)
const capabilities = ref<NativeIndexCapabilities | null>(null)
const archived = ref<NativeArchiveFilter>('unarchived')
const loading = ref(false)
const mutation = ref<{ id: string; action: 'open' | 'archive' } | null>(null)
const loaded = ref(false)
const error = ref('')
let read: AbortController | undefined
let watcher: AbortController | undefined
let actionRequest: AbortController | undefined
let refreshPending = false

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

function changeArchiveFilter() {
  sessions.value = []
  cursor.value = null
  loaded.value = false
  void load(false)
}

function canChangeArchive(session: NativeIndexedSession) {
  return !capabilities.value?.index || (session.state !== 'running' && session.state !== 'requires_action')
}

function dateLabel(value: string | null) {
  const timestamp = value ? Date.parse(value) : NaN
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : ''
}

watch(
  () => [props.workspace, props.agent],
  () => {
    watcher?.abort()
    watcher = undefined
    capabilities.value = null
    archived.value = 'unarchived'
    changeArchiveFilter()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  refreshPending = false
  read?.abort()
  watcher?.abort()
  actionRequest?.abort()
})
</script>

<template>
  <div class="space-y-3">
    <h3 class="text-xs font-medium text-muted-foreground">
      {{ t('chat.nativeAgent.history', { agent: externalAgentLabels[agent] }) }}
    </h3>
    <label v-if="capabilities?.archive" class="flex flex-wrap items-center gap-2 text-sm">
      <span>{{ t('chat.nativeAgent.archiveFilter') }}</span>
      <select
        v-model="archived"
        class="rounded-md border border-input bg-background px-2 py-1"
        :disabled="mutation !== null"
        @change="changeArchiveFilter"
      >
        <option value="unarchived">{{ t('chat.nativeAgent.unarchived') }}</option>
        <option value="archived">{{ t('chat.nativeAgent.archived') }}</option>
        <option value="all">{{ t('common.all') }}</option>
      </select>
    </label>
    <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
    <p v-else-if="loading && !loaded" role="status" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <p v-else-if="loaded && !sessions.length && cursor === null" class="text-sm text-muted-foreground">
      {{ t('chat.nativeAgent.noHistory') }}
    </p>
    <ul v-if="sessions.length" class="divide-y divide-border/70">
      <li v-for="session in sessions" :key="session.native_id" class="flex items-center gap-3 py-2.5">
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-medium">{{ session.title || session.native_id }}</p>
          <p v-if="dateLabel(session.updated_at)" class="truncate text-xs text-muted-foreground">
            {{ dateLabel(session.updated_at) }}
          </p>
        </div>
        <Button
          v-if="capabilities?.archive && session.archived !== null"
          variant="ghost"
          size="sm"
          :disabled="mutation !== null || !canChangeArchive(session)"
          @click="changeSession(session.native_id, 'archive', !session.archived)"
        >
          {{
            mutation?.id === session.native_id && mutation.action === 'archive'
              ? t('common.working')
              : t(session.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive')
          }}
        </Button>
        <Button
          variant="outline"
          size="sm"
          :disabled="mutation !== null"
          @click="changeSession(session.native_id, 'open')"
        >
          {{
            mutation?.id === session.native_id && mutation.action === 'open'
              ? t('common.working')
              : t('chat.nativeAgent.openHistory')
          }}
        </Button>
      </li>
    </ul>
    <div class="flex justify-end gap-2">
      <Button v-if="cursor !== null" variant="ghost" :disabled="loading || mutation !== null" @click="load(true)">
        {{ t('common.loadMore') }}
      </Button>
      <Button v-if="loaded || error" variant="ghost" :disabled="loading || mutation !== null" @click="load(false)">
        {{ t(loaded ? 'common.refresh' : 'common.retry') }}
      </Button>
    </div>
  </div>
</template>
