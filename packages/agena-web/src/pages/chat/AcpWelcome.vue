<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import {
  RiArrowRightUpLine,
  RiFolder6Line,
  RiHistoryLine,
  RiLoader4Line,
  RiSettings3Line,
  RiTerminalBoxLine,
} from '@remixicon/vue'

import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { acpAgentsSettingsDestination } from '@/components/settings/settingsDestination'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import {
  listExternalAgents,
  useExternalAgentCatalog,
} from '@/features/sessions/api/nativeAgentSessions'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { formatAcpCommandPreview } from '@/lib/acpAgentsConfig'
import { createSession } from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { fetchAgenaWorkspaces } from '@/stores/directorySessionStore'
import { useUiStore } from '@/stores/ui'
import { formatExternalAgentLabel, type ExternalAgent } from '@/types/chat'

defineProps<{
  loading?: boolean
  error?: string
}>()
const emit = defineEmits<{
  (event: 'open-sidebar'): void
}>()

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const ui = useUiStore()
const directory = useDirectoryStore()
const navigation = useWorkspaceNavigation()

const agents = useExternalAgentCatalog()
const projects = ref<DirectoryEntry[]>([])
const project = ref('')
const projectsLoading = ref(true)
const creatingAgent = ref<ExternalAgent | null>(null)
const launchError = ref('')
const lifetime = new AbortController()

const projectOptions = computed<PickerOption[]>(() =>
  projects.value.map((entry) => ({
    value: entry.id,
    label: entry.label || entry.path,
    description: entry.path,
    keywords: `${entry.label || ''} ${entry.path}`,
  })),
)

async function loadProjects() {
  projectsLoading.value = true
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
    const selected = entries.find((entry) => entry.path === directory.currentDirectory) || entries[0]
    project.value = selected?.id || ''
  } catch {
    // Sidebar remains available if workspace list fails here.
  } finally {
    projectsLoading.value = false
  }
}

async function startWithAgent(agentId: ExternalAgent) {
  if (creatingAgent.value || !project.value) return
  creatingAgent.value = agentId
  launchError.value = ''
  try {
    const session = await createSession({
      workspaceId: Number(project.value),
      externalAgent: agentId,
      title: formatExternalAgentLabel(agentId, agents.value),
      autoTitle: true,
    })
    if (lifetime.signal.aborted) return
    await navigation.openMainTab('acp', {
      path: '/acp',
      query: { sessionId: String(session.id) },
      replace: true,
    })
  } catch (failure) {
    if (!lifetime.signal.aborted) {
      launchError.value = failure instanceof Error ? failure.message : String(failure)
    }
  } finally {
    creatingAgent.value = null
  }
}

function openSettings() {
  const destination = acpAgentsSettingsDestination(route.query)
  if (ui.isCompactLayout) {
    ui.setSessionSwitcherOpen(false)
    void router.push(destination)
  } else {
    void navigation.openMainTab('settings', destination)
  }
}

onMounted(() => {
  void listExternalAgents(lifetime.signal).catch(() => {})
  void loadProjects()
})
onBeforeUnmount(() => lifetime.abort())
</script>

<template>
  <section
    class="flex h-full min-h-0 flex-col items-center justify-center overflow-y-auto px-4 py-8 sm:px-6"
    data-testid="acp-empty-view"
  >
    <div class="w-full max-w-2xl space-y-5">
      <div class="space-y-2">
        <div
          class="flex h-11 w-11 items-center justify-center rounded-2xl border border-primary/20 bg-primary/10 text-primary shadow-xs"
          aria-hidden="true"
        >
          <RiTerminalBoxLine class="h-5 w-5" />
        </div>
        <h1 class="text-xl font-semibold tracking-tight sm:text-2xl">{{ t('nav.acp') }}</h1>
        <p class="max-w-xl text-sm leading-relaxed text-muted-foreground">
          {{ t('chat.nativeAgent.welcomeDescription') }}
        </p>
      </div>

      <div
        v-if="loading"
        role="status"
        class="flex items-center gap-2 rounded-xl border border-border/70 bg-muted/20 px-4 py-3 text-sm text-muted-foreground"
      >
        <RiLoader4Line class="h-4 w-4 animate-spin text-primary" />
        <span>{{ t('common.loading') }}</span>
      </div>

      <div
        v-if="error || launchError"
        role="alert"
        class="break-words rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
      >
        {{ error || launchError }}
      </div>

      <div class="space-y-3 rounded-xl border border-border/70 bg-card/40 p-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="text-xs font-medium text-muted-foreground">{{ t('chat.nativeAgent.project') }}</span>
          <div class="w-full sm:w-72">
            <OptionPicker
              v-model="project"
              :options="projectOptions"
              :title="t('chat.nativeAgent.project')"
              :placeholder="t('chat.nativeAgent.project')"
              :search-placeholder="t('chat.nativeAgent.searchProjects')"
              :include-empty="false"
              :icon="RiFolder6Line"
              :disabled="projectsLoading || creatingAgent !== null || !projects.length"
              size="sm"
              trigger-class="w-full"
            />
          </div>
        </div>

        <p v-if="!projectsLoading && !projects.length" class="text-xs text-muted-foreground">
          {{ t('chat.nativeAgent.noProjects') }}
        </p>

        <div v-if="agents.length" class="grid gap-2.5 sm:grid-cols-2">
          <button
            v-for="entry in agents"
            :key="entry.id"
            type="button"
            class="group flex min-w-0 flex-col gap-1.5 rounded-xl border border-border/80 bg-background/60 p-3.5 text-left transition-colors hover:border-primary/40 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
            :disabled="!project || creatingAgent !== null"
            @click="startWithAgent(entry.id)"
          >
            <div class="flex items-center justify-between gap-2">
              <div class="flex min-w-0 items-center gap-2">
                <span class="truncate text-sm font-semibold text-foreground">{{ entry.name }}</span>
                <span
                  class="shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] font-medium"
                  :class="
                    entry.builtin
                      ? 'border-border/70 bg-muted/50 text-muted-foreground'
                      : 'border-primary/30 bg-primary/10 text-primary'
                  "
                >
                  {{ t(entry.builtin ? 'settings.acpAgents.status.builtin' : 'settings.acpAgents.status.custom') }}
                </span>
              </div>
              <RiLoader4Line v-if="creatingAgent === entry.id" class="h-4 w-4 shrink-0 animate-spin text-primary" />
              <RiArrowRightUpLine
                v-else
                class="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-primary"
              />
            </div>
            <p class="truncate font-mono text-[11px] text-muted-foreground">
              {{ formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id }}
            </p>
          </button>
        </div>
        <div
          v-else
          class="rounded-lg border border-dashed border-border/80 bg-muted/20 p-4 text-center text-xs text-muted-foreground"
        >
          {{ t('chat.nativeAgent.noEnabledAgents') }}
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" @click="emit('open-sidebar')">
          <RiHistoryLine class="mr-1.5 h-4 w-4" />
          {{ t('chat.nativeAgent.browseHistory') }}
        </Button>
        <Button size="sm" variant="ghost" @click="openSettings">
          <RiSettings3Line class="mr-1.5 h-4 w-4" />
          {{ t('chat.nativeAgent.configureAgents') }}
        </Button>
      </div>
    </div>
  </section>
</template>
