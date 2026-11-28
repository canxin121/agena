<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import {
  RiAddLine,
  RiEqualizerLine,
  RiFolder6Line,
  RiLoader4Line,
  RiRefreshLine,
  RiSettings3Line,
  RiTerminalBoxLine,
} from '@remixicon/vue'

import { readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { acpAgentsSettingsDestination } from '@/components/settings/settingsDestination'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import {
  listExternalAgents,
  listOpenAcpSessions,
  useExternalAgentCatalog,
  type OpenAcpSession,
} from '@/features/sessions/api/nativeAgentSessions'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { formatAcpCommandPreview } from '@/lib/acpAgentsConfig'
import { isAcpSessionId } from '@/lib/acpSession'
import { useChatStoreUnscoped } from '@/stores/chat'
import { createSession } from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { fetchAgenaWorkspaces } from '@/stores/directorySessionStore'
import { useUiStore } from '@/stores/ui'
import { formatExternalAgentLabel, type ExternalAgent } from '@/types/chat'
import ExternalAgentHistoryList from './ExternalAgentHistoryList.vue'
import ExternalAgentSessionDialog from './chatSidebar/components/ExternalAgentSessionDialog.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const ui = useUiStore()
const chat = useChatStoreUnscoped()
const directory = useDirectoryStore()
const navigation = useWorkspaceNavigation()
const projects = ref<DirectoryEntry[]>([])
const project = ref('')
const agents = useExternalAgentCatalog()
const agent = ref<ExternalAgent>(agents.value[0]?.id || 'claude-code')
const creating = ref(false)
const loading = ref(true)
const error = ref('')
const managed = ref('')
const historyRevision = ref(0)
const openViews = ref<OpenAcpSession[]>([])
const openViewsLoading = ref(false)
const lifetime = new AbortController()
let openViewsRequest: AbortController | undefined
const currentId = computed(() => readSessionIdFromQuery(route.query))
const current = computed(() => (isAcpSessionId(currentId.value) ? chat.getSessionById(currentId.value) : null))
const agentLabel = (id: ExternalAgent) => formatExternalAgentLabel(id, agents.value)
const activeAgentEntry = computed(() => agents.value.find((entry) => entry.id === agent.value))

const projectOptions = computed<PickerOption[]>(() =>
  projects.value.map((entry) => ({
    value: entry.id,
    label: entry.label || entry.path,
    description: entry.path,
    keywords: `${entry.label || ''} ${entry.path}`,
  })),
)

const agentOptions = computed<PickerOption[]>(() =>
  agents.value.map((entry) => {
    const commandPreview = formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id
    return {
      value: entry.id,
      label: entry.name,
      description:
        entry.available === false
          ? `${t('chat.nativeAgent.installGuide.notInstalledBadge')} · ${commandPreview}`
          : commandPreview,
      group: String(t(entry.builtin ? 'settings.acpAgents.status.builtin' : 'settings.acpAgents.status.custom')),
      keywords: `${entry.id} ${entry.name} ${entry.command}`,
    }
  }),
)

const showAgentPills = computed(() => agents.value.length > 0 && agents.value.length <= 4)

function openAcpSettings() {
  const destination = acpAgentsSettingsDestination(route.query)
  if (ui.isCompactLayout) {
    ui.setSessionSwitcherOpen(false)
    void router.push(destination)
  } else {
    void navigation.openMainTab('settings', destination)
  }
}

async function loadAgents() {
  try {
    const items = await listExternalAgents(lifetime.signal)
    if (lifetime.signal.aborted) return
    if (items.length && !items.some((entry) => entry.id === agent.value)) {
      agent.value = items[0].id
    }
  } catch {
    // Keep built-in defaults if the catalog request fails.
  }
}

async function loadOpenViews() {
  openViewsRequest?.abort()
  if (!project.value) {
    openViews.value = []
    return
  }
  const request = new AbortController()
  openViewsRequest = request
  openViewsLoading.value = true
  try {
    const items = await listOpenAcpSessions(Number(project.value), request.signal)
    if (!request.signal.aborted && !lifetime.signal.aborted) openViews.value = items
  } catch {
    if (!request.signal.aborted && !lifetime.signal.aborted) openViews.value = []
  } finally {
    if (openViewsRequest === request) openViewsLoading.value = false
  }
}

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
    if (current.value?.external_agent && agents.value.some((entry) => entry.id === current.value?.external_agent)) {
      agent.value = current.value.external_agent
    }
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
  void loadOpenViews()
}

async function newConversation() {
  if (creating.value || !project.value || !agent.value) return
  creating.value = true
  error.value = ''
  try {
    const session = await createSession({
      workspaceId: Number(project.value),
      externalAgent: agent.value,
      title: agentLabel(agent.value),
      autoTitle: true,
    })
    await openView(session.id)
  } catch (failure) {
    if (!lifetime.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    creating.value = false
  }
}

watch([project, historyRevision, currentId, () => current.value?.title], () => void loadOpenViews())
watch(
  () => current.value?.external_agent,
  (nextAgent) => {
    if (nextAgent && agents.value.some((entry) => entry.id === nextAgent)) {
      agent.value = nextAgent
    }
  },
)
onMounted(() => {
  void loadAgents()
  void loadProjects()
})
onBeforeUnmount(() => {
  openViewsRequest?.abort()
  lifetime.abort()
})
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground" :aria-label="t('nav.acp')">
    <div class="space-y-2.5 border-b border-sidebar-border px-3 py-3">
      <div class="flex items-center justify-between gap-2">
        <div class="flex min-w-0 items-center gap-1.5">
          <RiTerminalBoxLine class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
          <h2 class="truncate text-sm font-semibold tracking-tight">{{ t('nav.acp') }}</h2>
        </div>
        <div class="flex shrink-0 items-center gap-1">
          <IconButton
            size="sm"
            variant="ghost"
            :tooltip="t('chat.nativeAgent.configureAgents')"
            :aria-label="t('chat.nativeAgent.configureAgents')"
            :is-touch-pointer="ui.isTouchPointer"
            @click="openAcpSettings"
          >
            <RiSettings3Line class="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      <div class="space-y-1">
        <span class="block text-[11px] font-medium text-muted-foreground">{{ t('chat.nativeAgent.project') }}</span>
        <OptionPicker
          v-model="project"
          :options="projectOptions"
          :title="t('chat.nativeAgent.project')"
          :placeholder="t('chat.nativeAgent.project')"
          :search-placeholder="t('chat.nativeAgent.searchProjects')"
          :include-empty="false"
          :icon="RiFolder6Line"
          :disabled="loading || creating || !projects.length"
          size="sm"
          trigger-class="w-full"
        />
      </div>

      <div class="space-y-1">
        <span class="block text-[11px] font-medium text-muted-foreground">{{ t('chat.nativeAgent.agent') }}</span>
        <div
          v-if="showAgentPills"
          role="radiogroup"
          :aria-label="t('chat.nativeAgent.agent')"
          class="flex flex-wrap gap-1 rounded-md border border-sidebar-border/80 bg-sidebar-accent/25 p-1"
        >
          <button
            v-for="entry in agents"
            :key="entry.id"
            type="button"
            role="radio"
            :aria-checked="agent === entry.id"
            :title="
              entry.available === false
                ? `${t('chat.nativeAgent.installGuide.notInstalledBadge')} · ${formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id}`
                : formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id
            "
            :disabled="creating"
            class="flex min-w-0 flex-1 basis-[calc(50%-0.125rem)] items-center justify-center gap-1.5 rounded px-2 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
            :class="
              agent === entry.id
                ? 'bg-background text-foreground shadow-xs ring-1 ring-border/70'
                : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground'
            "
            @click="agent = entry.id"
          >
            <span
              v-if="entry.available === false"
              class="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500"
              aria-hidden="true"
            />
            <span
              v-else-if="!entry.builtin"
              class="h-1.5 w-1.5 shrink-0 rounded-full bg-primary"
              aria-hidden="true"
            />
            <span class="truncate">{{ entry.name }}</span>
          </button>
        </div>
        <OptionPicker
          v-else-if="agents.length > 4"
          v-model="agent"
          :options="agentOptions"
          :title="t('chat.nativeAgent.agent')"
          :placeholder="t('chat.nativeAgent.agent')"
          :search-placeholder="t('chat.nativeAgent.searchAgents')"
          :include-empty="false"
          :disabled="creating"
          size="sm"
          trigger-class="w-full"
        />
        <div
          v-else
          class="space-y-2 rounded-md border border-dashed border-border/80 bg-muted/20 p-2.5 text-xs text-muted-foreground"
        >
          <p>{{ t('chat.nativeAgent.noEnabledAgents') }}</p>
          <Button size="sm" variant="outline" class="w-full" @click="openAcpSettings">
            <RiSettings3Line class="mr-1.5 h-3.5 w-3.5" />
            {{ t('chat.nativeAgent.configureAgents') }}
          </Button>
        </div>
      </div>

      <div class="flex items-center gap-1.5">
        <Button
          size="sm"
          class="min-w-0 flex-1"
          :disabled="!project || creating || !agents.length"
          @click="newConversation"
        >
          <RiLoader4Line v-if="creating" class="mr-1.5 h-3.5 w-3.5 shrink-0 animate-spin" />
          <RiAddLine v-else class="mr-1.5 h-3.5 w-3.5 shrink-0" />
          <span class="truncate">
            {{
              creating
                ? t('common.working')
                : activeAgentEntry
                  ? t('chat.nativeAgent.newWithAgent', { agent: activeAgentEntry.name })
                  : t('chat.sidebar.directoryActions.newSession.label')
            }}
          </span>
        </Button>
        <IconButton
          v-if="current"
          size="sm"
          variant="outline"
          :tooltip="t('chat.nativeAgent.manage')"
          :aria-label="t('chat.nativeAgent.manage')"
          :is-touch-pointer="ui.isTouchPointer"
          @click="managed = current.id"
        >
          <RiEqualizerLine class="h-4 w-4" />
        </IconButton>
      </div>

      <p v-if="error" class="break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive" role="alert">
        {{ error }}
      </p>
      <Button v-if="error && !projects.length" variant="ghost" size="sm" class="w-full" @click="loadProjects">
        {{ t('common.retry') }}
      </Button>
      <p v-else-if="loading" class="text-xs text-muted-foreground" role="status">{{ t('common.loading') }}</p>
      <p v-else-if="!projects.length && !error" class="text-xs text-muted-foreground">
        {{ t('chat.nativeAgent.noProjects') }}
      </p>
    </div>

    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto px-3 py-3">
      <section v-if="openViews.length" class="space-y-1.5" :aria-label="t('chat.nativeAgent.openViews')">
        <div class="flex items-center justify-between gap-2 px-1">
          <div class="flex items-center gap-1.5">
            <h3 class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              {{ t('chat.nativeAgent.openViews') }}
            </h3>
            <span class="rounded-full bg-muted/70 px-1.5 py-0.2 text-[10px] font-medium tabular-nums text-muted-foreground">
              {{ openViews.length }}
            </span>
          </div>
          <IconButton
            size="sm"
            variant="ghost"
            class="h-6 w-6"
            :disabled="openViewsLoading"
            :tooltip="t('common.refresh')"
            :aria-label="t('common.refresh')"
            @click="loadOpenViews"
          >
            <RiRefreshLine class="h-3.5 w-3.5" :class="openViewsLoading ? 'animate-spin' : ''" />
          </IconButton>
        </div>
        <ul class="space-y-0.5">
          <li v-for="view in openViews" :key="view.id">
            <button
              type="button"
              class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-sidebar-accent/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :class="String(view.id) === currentId ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground' : ''"
              @click="openView(view.id)"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full"
                :class="String(view.id) === currentId ? 'bg-primary' : 'bg-muted-foreground/40'"
                aria-hidden="true"
              />
              <span class="min-w-0 flex-1 truncate">{{ view.title || `#${view.id}` }}</span>
              <span
                v-if="view.external_agent"
                class="shrink-0 rounded bg-muted/60 px-1.5 py-0.5 text-[10px] text-muted-foreground"
              >
                {{ agentLabel(view.external_agent) }}
              </span>
            </button>
          </li>
        </ul>
      </section>

      <ExternalAgentHistoryList
        v-if="project && agent"
        :key="`${project}:${agent}:${historyRevision}`"
        :workspace="Number(project)"
        :agent="agent"
        :agent-label="agentLabel(agent)"
        @opened="openView"
      />
    </div>

    <ExternalAgentSessionDialog
      v-if="managed"
      :session="managed"
      :agent="current?.external_agent"
      @close="managed = ''"
      @changed="historyRevision++"
      @deleted="void openView()"
      @forked="openView"
    />
  </aside>
</template>
