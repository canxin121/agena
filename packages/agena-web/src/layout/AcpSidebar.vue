<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import Button from '@/components/ui/Button.vue'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { isAcpSessionId } from '@/lib/acpSession'
import { useChatStoreUnscoped } from '@/stores/chat'
import { createSession } from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { fetchAgenaWorkspaces } from '@/stores/directorySessionStore'
import { useUiStore } from '@/stores/ui'
import { externalAgentLabels, type ExternalAgent } from '@/types/chat'
import ExternalAgentHistoryList from './ExternalAgentHistoryList.vue'
import ExternalAgentSessionDialog from './chatSidebar/components/ExternalAgentSessionDialog.vue'

const { t } = useI18n()
const route = useRoute()
const ui = useUiStore()
const chat = useChatStoreUnscoped()
const directory = useDirectoryStore()
const navigation = useWorkspaceNavigation()
const projects = ref<DirectoryEntry[]>([])
const project = ref('')
const agent = ref<ExternalAgent>('claude-code')
const creating = ref(false)
const loading = ref(true)
const error = ref('')
const managed = ref('')
const historyRevision = ref(0)
const lifetime = new AbortController()
const currentId = computed(() => readSessionIdFromQuery(route.query))
const current = computed(() => (isAcpSessionId(currentId.value) ? chat.getSessionById(currentId.value) : null))

async function loadProjects() {
  error.value = ''
  loading.value = true
  try {
    const entries: DirectoryEntry[] = []
    let page = 0
    let hasMore = true
    while (hasMore && !lifetime.signal.aborted) {
      const result = await fetchAgenaWorkspaces({ limit: 200, page: page++, signal: lifetime.signal })
      entries.push(...result.entries)
      hasMore = result.hasMore
    }
    if (lifetime.signal.aborted) return
    projects.value = entries
    const selected =
      entries.find((entry) => entry.id === String(current.value?.workspace_id)) ||
      entries.find((entry) => entry.path === directory.currentDirectory) ||
      entries[0]
    project.value = selected?.id || ''
    if (current.value?.external_agent) agent.value = current.value.external_agent
  } catch (failure) {
    if (!lifetime.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    loading.value = false
  }
}

async function openView(id?: string | number) {
  if (lifetime.signal.aborted || ui.activeMainTab !== 'acp') return
  managed.value = ''
  await navigation.openMainTab('acp', {
    path: '/acp',
    query: id ? { sessionId: String(id) } : {},
    replace: true,
  })
  if (id) ui.setSessionSwitcherOpen(false)
}

async function newConversation() {
  if (creating.value || !project.value) return
  creating.value = true
  error.value = ''
  try {
    const session = await createSession({
      workspaceId: Number(project.value),
      externalAgent: agent.value,
      title: externalAgentLabels[agent.value],
    })
    await openView(session.id)
  } catch (failure) {
    if (!lifetime.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    creating.value = false
  }
}

onMounted(() => void loadProjects())
onBeforeUnmount(() => lifetime.abort())
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground" :aria-label="t('nav.acp')">
    <div class="space-y-3 border-b border-sidebar-border px-4 py-3">
      <h2 class="text-sm font-semibold">{{ t('nav.acp') }}</h2>
      <label class="flex flex-col gap-1 text-xs">
        {{ t('chat.nativeAgent.project') }}
        <select
          v-model="project"
          :aria-label="t('chat.nativeAgent.project')"
          class="min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-sm"
          :disabled="loading || creating || !projects.length"
        >
          <option v-for="entry in projects" :key="entry.id" :value="entry.id" :title="entry.path">
            {{ entry.label || entry.path }}
          </option>
        </select>
      </label>
      <label class="flex flex-col gap-1 text-xs">
        {{ t('chat.nativeAgent.agent') }}
        <select
          v-model="agent"
          :aria-label="t('chat.nativeAgent.agent')"
          class="rounded-md border border-input bg-background px-2 py-1.5 text-sm"
          :disabled="creating"
        >
          <option v-for="(label, value) in externalAgentLabels" :key="value" :value="value">{{ label }}</option>
        </select>
      </label>
      <div class="flex flex-wrap gap-2">
        <Button size="sm" :disabled="!project || creating" @click="newConversation">
          {{ t(creating ? 'common.working' : 'chat.sidebar.directoryActions.newSession.label') }}
        </Button>
        <Button v-if="current" size="sm" variant="outline" @click="managed = current.id">
          {{ t('chat.nativeAgent.manage') }}
        </Button>
      </div>
      <p v-if="error" class="text-xs text-destructive" role="alert">{{ error }}</p>
      <Button v-if="error && !projects.length" variant="ghost" size="sm" @click="loadProjects">
        {{ t('common.retry') }}
      </Button>
      <p v-else-if="loading" class="text-xs text-muted-foreground" role="status">{{ t('common.loading') }}</p>
      <p v-else-if="!projects.length && !error" class="text-xs text-muted-foreground">
        {{ t('chat.nativeAgent.noProjects') }}
      </p>
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
      <ExternalAgentHistoryList
        v-if="project"
        :key="`${project}:${agent}:${historyRevision}`"
        :workspace="Number(project)"
        :agent="agent"
        @opened="openView"
      />
    </div>
    <ExternalAgentSessionDialog
      v-if="managed"
      :session="managed"
      @close="managed = ''"
      @changed="historyRevision++"
      @deleted="void openView()"
      @forked="openView"
    />
  </aside>
</template>
