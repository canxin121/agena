<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiAddLine,
  RiCheckLine,
  RiCloseLine,
  RiFileCopyLine,
  RiFolder6Line,
  RiLoader4Line,
  RiRefreshLine,
  RiRouteLine,
  RiShieldUserLine,
} from '@remixicon/vue'

import AcpInstallGuideCard from '@/components/acp/AcpInstallGuideCard.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import {
  getExternalAgentProviders,
  listExternalAgents,
  useExternalAgentCatalog,
  type ExternalAgentProviders,
} from '@/features/sessions/api/nativeAgentSessions'
import { inferAcpAgentIdFromError, isMissingAcpCommandError } from '@/lib/acpInstallGuide'
import { sessionApiPath } from '@/lib/acpSession'
import { apiJson } from '@/lib/api'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  MAX_ACP_ADDITIONAL_DIRECTORIES,
  normalizeAdditionalDirectories,
} from '@/pages/chat/acpSessionControls'
import { useToastsStore } from '@/stores/toasts'
import type { ExternalAgent, ExternalAgentAuthStatus } from '@/types/chat'

type PanelTab = 'directories' | 'account' | 'providers'

type AgentAuth = Partial<ExternalAgentAuthStatus> & {
  command?: string
  base_args?: string[]
  methods: { id: string; name: string; description?: string; type: 'agent' | 'terminal' }[]
  logout_supported: boolean
}
type AuthAction = { action: 'inspect' | 'logout' } | { action: 'login'; method_id: string }

const props = defineProps<{
  sessionId: string
  agent: ExternalAgent
  agentLabel: string
  supportsDirectories: boolean
  additionalDirectories: readonly string[]
  directoriesSaving?: boolean
  updateDirectories: (directories: string[]) => Promise<string[]>
}>()

const { t } = useI18n()
const toasts = useToastsStore()
const catalog = useExternalAgentCatalog()

const activeTab = ref<PanelTab | null>(null)

// Account / auth state
const authBusy = ref(false)
const authError = ref('')
const auth = ref<AgentAuth>()
const liveAuth = ref<ExternalAgentAuthStatus | null>(null)
const copiedCommand = ref('')
let copyTimer: number | undefined

// Provider routing state
const providersBusy = ref(false)
const providersError = ref('')
const providers = ref<ExternalAgentProviders | null>(null)

// Additional workspace directories state
const directoryDraft = ref('')
const directoriesError = ref('')

const hasSession = computed(() => Boolean(props.sessionId))
const directoriesList = computed(() => props.additionalDirectories ?? [])
const canAddMoreDirectories = computed(
  () => directoriesList.value.length < MAX_ACP_ADDITIONAL_DIRECTORIES,
)

const resolvedAgentId = computed(
  () => props.agent || inferAcpAgentIdFromError(authError.value) || 'custom',
)
const resolvedAgentEntry = computed(() => catalog.value.find((entry) => entry.id === resolvedAgentId.value))
const showAuthInstallGuide = computed(() => Boolean(authError.value) && isMissingAcpCommandError(authError.value))

const displayedAuth = computed(() => liveAuth.value ?? auth.value)
const authPath = computed(() => (props.sessionId ? `${sessionApiPath(props.sessionId)}/agent-auth` : ''))
const authStateKey = computed(
  () =>
    ({
      authenticated: 'agentSignedIn',
      unauthenticated: 'agentSignInRequired',
      not_required: 'agentSignInNotRequired',
      unknown: 'agentStatusUnavailable',
    })[displayedAuth.value?.state ?? 'unknown'],
)
const authDotClass = computed(() => {
  const state = displayedAuth.value?.state
  if (state === 'authenticated') return 'bg-emerald-500'
  if (state === 'unauthenticated') return 'bg-amber-500'
  return 'bg-muted-foreground/40'
})
const agentLoginMethods = computed(() => (auth.value?.methods ?? []).filter((method) => method.type === 'agent'))
const terminalLoginMethods = computed(() => (auth.value?.methods ?? []).filter((method) => method.type === 'terminal'))
const terminalInspectCommand = computed(() =>
  props.sessionId ? `agena sessions agent-auth ${props.sessionId} --terminal` : '',
)
const routedProviderCount = computed(
  () => providers.value?.providers.filter((provider) => provider.current !== null).length ?? 0,
)

function toggleTab(tab: PanelTab) {
  if (activeTab.value === tab) {
    activeTab.value = null
    return
  }
  activeTab.value = tab
  if (tab === 'account' && props.sessionId) void runAuth()
  if (tab === 'providers' && props.sessionId) void loadProviders()
}

async function runAuth(action: AuthAction = { action: 'inspect' }) {
  if (authBusy.value || !props.sessionId) return
  authBusy.value = true
  authError.value = ''
  try {
    auth.value = await apiJson<AgentAuth>(authPath.value, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(action),
    })
  } catch (reason) {
    authError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    authBusy.value = false
  }
}

async function loadProviders() {
  if (providersBusy.value || !props.sessionId) return
  providersBusy.value = true
  providersError.value = ''
  try {
    providers.value = await getExternalAgentProviders(props.sessionId)
  } catch (reason) {
    providersError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    providersBusy.value = false
  }
}

async function addAdditionalDirectory() {
  const path = directoryDraft.value.trim()
  if (!path || !props.sessionId || props.directoriesSaving) return
  directoriesError.value = ''
  const next = normalizeAdditionalDirectories([...directoriesList.value, path])
  if (next.length === directoriesList.value.length && directoriesList.value.includes(path)) {
    directoryDraft.value = ''
    return
  }
  try {
    await props.updateDirectories(next)
    directoryDraft.value = ''
    toasts.push('success', String(t('chat.nativeAgent.directoriesSavedToast')))
  } catch (reason) {
    directoriesError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function removeAdditionalDirectory(directory: string) {
  if (!props.sessionId || props.directoriesSaving) return
  directoriesError.value = ''
  const next = directoriesList.value.filter((item) => item !== directory)
  try {
    await props.updateDirectories(next)
    toasts.push('success', String(t('chat.nativeAgent.directoriesSavedToast')))
  } catch (reason) {
    directoriesError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function copyCommand(command: string) {
  const ok = await copyTextToClipboard(command)
  if (!ok) return
  copiedCommand.value = command
  toasts.push('success', String(t('common.copied')))
  window.clearTimeout(copyTimer)
  copyTimer = window.setTimeout(() => {
    if (copiedCommand.value === command) copiedCommand.value = ''
  }, 1800)
}

// Reset panel data when switching agent or session.
watch(
  () => [props.agent, props.sessionId] as const,
  ([nextAgent, nextSid], [prevAgent, prevSid]) => {
    if (nextAgent !== prevAgent) {
      activeTab.value = null
      auth.value = undefined
      liveAuth.value = null
      authError.value = ''
      providers.value = null
      providersError.value = ''
      directoriesError.value = ''
      return
    }
    if (nextSid !== prevSid) {
      auth.value = undefined
      liveAuth.value = null
      authError.value = ''
      providers.value = null
      providersError.value = ''
      directoriesError.value = ''
      if (activeTab.value === 'account' && nextSid) void runAuth()
      if (activeTab.value === 'providers' && nextSid) void loadProviders()
    }
  },
)

// Close directories tab if active session does not advertise additionalDirectories.
watch(
  () => props.supportsDirectories,
  (supported) => {
    if (!supported && activeTab.value === 'directories') {
      activeTab.value = null
    }
  },
)

// Poll live auth status while the Account drawer is open.
watch(
  [activeTab, authPath],
  ([tab, path], _previous, onCleanup) => {
    if (tab !== 'account' || !path) {
      liveAuth.value = null
      return
    }
    const controller = new AbortController()
    let refreshing = false
    async function refresh() {
      if (refreshing || controller.signal.aborted) return
      refreshing = true
      try {
        const status = await apiJson<ExternalAgentAuthStatus | null>(path, { signal: controller.signal })
        if (!controller.signal.aborted) liveAuth.value = status
      } catch {
        if (!controller.signal.aborted) liveAuth.value = null
      } finally {
        refreshing = false
      }
    }
    void refresh()
    const timer = window.setInterval(refresh, 1500)
    onCleanup(() => {
      controller.abort()
      window.clearInterval(timer)
    })
  },
)

onBeforeUnmount(() => {
  window.clearTimeout(copyTimer)
})
</script>

<template>
  <div class="shrink-0 border-t border-sidebar-border/60 bg-sidebar/95">
    <!-- Expanded drawer above footer bar -->
    <div
      v-if="activeTab"
      class="max-h-72 overflow-y-auto border-b border-sidebar-border/60 bg-background/50 px-3 py-2.5 text-xs"
      @keydown.esc.stop="activeTab = null"
    >
      <!-- Directories panel -->
      <div v-if="activeTab === 'directories'" class="grid gap-2" :aria-busy="directoriesSaving">
        <div class="flex items-center justify-between gap-2">
          <span class="font-medium text-foreground">{{ t('chat.nativeAgent.directories') }}</span>
          <IconButton
            size="sm"
            variant="ghost"
            class="h-6 w-6 shrink-0"
            :aria-label="t('common.close')"
            @click="activeTab = null"
          >
            <RiCloseLine class="h-3.5 w-3.5" />
          </IconButton>
        </div>

        <p v-if="!hasSession" class="text-[11px] text-muted-foreground">
          {{ t('chat.nativeAgent.sidebarToolsNeedSession') }}
        </p>

        <template v-else>
          <p
            v-if="directoriesError"
            role="alert"
            class="whitespace-pre-wrap break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-destructive"
          >
            {{ directoriesError }}
          </p>

          <ul v-if="directoriesList.length" class="grid gap-1">
            <li
              v-for="dir in directoriesList"
              :key="dir"
              class="flex items-center gap-1.5 rounded-md border border-border/70 bg-muted/20 px-2 py-1"
            >
              <span class="min-w-0 flex-1 break-all font-mono text-[11px] text-foreground">{{ dir }}</span>
              <IconButton
                size="sm"
                variant="ghost"
                class="h-5 w-5 shrink-0 text-muted-foreground hover:text-destructive"
                :disabled="directoriesSaving"
                :tooltip="t('chat.nativeAgent.removeDirectoryAction')"
                :aria-label="t('chat.nativeAgent.removeDirectoryAction')"
                @click="removeAdditionalDirectory(dir)"
              >
                <RiCloseLine class="h-3.5 w-3.5" />
              </IconButton>
            </li>
          </ul>
          <p v-else class="text-[11px] text-muted-foreground">
            {{ t('chat.nativeAgent.noAdditionalDirectories') }}
          </p>

          <form
            v-if="canAddMoreDirectories"
            class="flex items-center gap-1.5"
            @submit.prevent="addAdditionalDirectory"
          >
            <input
              v-model="directoryDraft"
              type="text"
              class="h-7 min-w-0 flex-1 rounded-md border border-border/70 bg-background px-2 font-mono text-[11px] text-foreground outline-none focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-60"
              :placeholder="t('chat.nativeAgent.addDirectoryPlaceholder')"
              :aria-label="t('chat.nativeAgent.directories')"
              :disabled="directoriesSaving"
              autocomplete="off"
              spellcheck="false"
            />
            <Button
              type="submit"
              size="sm"
              variant="outline"
              class="h-7 shrink-0 px-2 text-[11px]"
              :disabled="directoriesSaving || !directoryDraft.trim()"
            >
              <RiLoader4Line v-if="directoriesSaving" class="mr-1 h-3 w-3 animate-spin" />
              <RiAddLine v-else class="mr-0.5 h-3 w-3" />
              {{ t('chat.nativeAgent.addDirectoryAction') }}
            </Button>
          </form>

          <p class="text-[11px] text-muted-foreground">{{ t('chat.nativeAgent.directoriesHint') }}</p>
        </template>
      </div>

      <!-- Account / Auth panel -->
      <div v-else-if="activeTab === 'account'" class="grid gap-2" :aria-busy="authBusy">
        <div class="flex items-center justify-between gap-2">
          <span class="font-medium text-foreground">{{ t('chat.composer.picker.agentAccount') }}</span>
          <div class="flex items-center gap-0.5">
            <IconButton
              v-if="hasSession"
              size="sm"
              variant="ghost"
              class="h-6 w-6 shrink-0"
              :disabled="authBusy"
              :tooltip="t('common.refresh')"
              :aria-label="t('common.refresh')"
              @click="runAuth()"
            >
              <RiRefreshLine class="h-3.5 w-3.5" :class="authBusy ? 'animate-spin' : ''" />
            </IconButton>
            <IconButton
              size="sm"
              variant="ghost"
              class="h-6 w-6 shrink-0"
              :aria-label="t('common.close')"
              @click="activeTab = null"
            >
              <RiCloseLine class="h-3.5 w-3.5" />
            </IconButton>
          </div>
        </div>

        <p v-if="!hasSession" class="text-[11px] text-muted-foreground">
          {{ t('chat.nativeAgent.sidebarToolsNeedSession') }}
        </p>

        <template v-else>
          <AcpInstallGuideCard
            v-if="showAuthInstallGuide"
            compact
            :agent-id="resolvedAgentId"
            :agent-name="resolvedAgentEntry?.name || agentLabel"
            :command="resolvedAgentEntry?.command"
            :raw-error="authError"
            :show-retry="true"
            :retrying="authBusy"
            @configure="activeTab = null"
            @retry="() => { void listExternalAgents().catch(() => {}); void runAuth() }"
          />
          <p
            v-else-if="authError"
            role="alert"
            class="whitespace-pre-wrap break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-destructive"
          >
            {{ authError }}
          </p>
          <p v-else-if="authBusy && !displayedAuth" role="status" class="text-muted-foreground">
            {{ t('chat.composer.status.loading') }}
          </p>

          <div
            v-if="displayedAuth"
            class="space-y-1 rounded-md border border-border/60 bg-muted/20 px-2.5 py-2"
          >
            <p role="status" class="flex items-center gap-1.5 font-medium text-foreground">
              <span class="h-2 w-2 shrink-0 rounded-full" :class="authDotClass" aria-hidden="true" />
              <span v-if="liveAuth" class="text-muted-foreground">{{ t('chat.composer.picker.agentConnectionStatus') }} ·</span>
              <span>{{ t(`chat.composer.picker.${authStateKey}`) }}</span>
            </p>
            <div
              v-for="(account, index) in displayedAuth.accounts"
              :key="index"
              class="break-words text-[11px] text-muted-foreground"
            >
              <p class="flex flex-wrap items-center gap-1.5 text-foreground">
                <span>{{ [account.label, account.name, account.email, account.organization].filter(Boolean).join(' · ') }}</span>
                <span
                  v-if="account.active"
                  class="rounded-full border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.2 text-[10px] font-medium text-emerald-700 dark:text-emerald-300"
                >
                  {{ t('chat.composer.picker.agentAccountActive') }}
                </span>
              </p>
              <p v-if="account.description">{{ account.description }}</p>
            </div>
          </div>

          <template v-if="auth">
            <div v-if="agentLoginMethods.length || auth.logout_supported" class="flex flex-wrap gap-1.5">
              <Button
                v-for="method in agentLoginMethods"
                :key="method.id"
                size="sm"
                variant="outline"
                :disabled="authBusy"
                :title="method.description"
                @click="runAuth({ action: 'login', method_id: method.id })"
              >
                {{ method.name }}
              </Button>
              <Button
                v-if="auth.logout_supported"
                size="sm"
                variant="ghost"
                class="text-destructive hover:bg-destructive/10 hover:text-destructive"
                :disabled="authBusy"
                @click="runAuth({ action: 'logout' })"
              >
                {{ t('chat.composer.picker.agentSignOut') }}
              </Button>
            </div>

            <div class="space-y-1.5 rounded-md border border-border/60 bg-muted/20 px-2.5 py-2">
              <p class="text-[11px] text-muted-foreground">{{ t('chat.composer.picker.agentTerminalSignIn') }}</p>
              <div v-if="terminalLoginMethods.length" class="space-y-1.5">
                <div v-for="method in terminalLoginMethods" :key="method.id" class="space-y-1">
                  <p class="text-[11px] font-medium text-foreground">
                    {{ method.name }}
                    <span v-if="method.description" class="font-normal text-muted-foreground"> · {{ method.description }}</span>
                  </p>
                  <div class="flex items-center gap-1 rounded border border-border/70 bg-background px-2 py-1">
                    <code class="min-w-0 flex-1 select-all break-all font-mono text-[11px]">
                      agena sessions agent-auth {{ sessionId }} --terminal --login {{ method.id }}
                    </code>
                    <IconButton
                      size="sm"
                      variant="ghost"
                      class="h-6 w-6 shrink-0"
                      :tooltip="t('common.copy')"
                      :aria-label="t('common.copy')"
                      @click="copyCommand(`agena sessions agent-auth ${sessionId} --terminal --login ${method.id}`)"
                    >
                      <RiCheckLine
                        v-if="copiedCommand === `agena sessions agent-auth ${sessionId} --terminal --login ${method.id}`"
                        class="h-3.5 w-3.5 text-emerald-500"
                      />
                      <RiFileCopyLine v-else class="h-3.5 w-3.5" />
                    </IconButton>
                  </div>
                </div>
              </div>
              <div v-else class="flex items-center gap-1 rounded border border-border/70 bg-background px-2 py-1">
                <code class="min-w-0 flex-1 select-all break-all font-mono text-[11px]">{{ terminalInspectCommand }}</code>
                <IconButton
                  size="sm"
                  variant="ghost"
                  class="h-6 w-6 shrink-0"
                  :tooltip="t('common.copy')"
                  :aria-label="t('common.copy')"
                  @click="copyCommand(terminalInspectCommand)"
                >
                  <RiCheckLine v-if="copiedCommand === terminalInspectCommand" class="h-3.5 w-3.5 text-emerald-500" />
                  <RiFileCopyLine v-else class="h-3.5 w-3.5" />
                </IconButton>
              </div>
            </div>
          </template>
        </template>
      </div>

      <!-- Provider routing panel -->
      <div v-else-if="activeTab === 'providers'" class="grid gap-2" :aria-busy="providersBusy">
        <div class="flex items-center justify-between gap-2">
          <span class="font-medium text-foreground">{{ t('chat.composer.picker.agentProviders') }}</span>
          <div class="flex items-center gap-0.5">
            <IconButton
              v-if="hasSession"
              size="sm"
              variant="ghost"
              class="h-6 w-6 shrink-0"
              :disabled="providersBusy"
              :tooltip="t('common.refresh')"
              :aria-label="t('common.refresh')"
              @click="loadProviders"
            >
              <RiRefreshLine class="h-3.5 w-3.5" :class="providersBusy ? 'animate-spin' : ''" />
            </IconButton>
            <IconButton
              size="sm"
              variant="ghost"
              class="h-6 w-6 shrink-0"
              :aria-label="t('common.close')"
              @click="activeTab = null"
            >
              <RiCloseLine class="h-3.5 w-3.5" />
            </IconButton>
          </div>
        </div>

        <p v-if="!hasSession" class="text-[11px] text-muted-foreground">
          {{ t('chat.nativeAgent.sidebarToolsNeedSession') }}
        </p>

        <template v-else>
          <p
            v-if="providersError"
            role="alert"
            class="whitespace-pre-wrap break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-destructive"
          >
            {{ providersError }}
          </p>
          <p v-else-if="providersBusy" role="status" class="text-muted-foreground">
            {{ t('chat.composer.status.loading') }}
          </p>
          <template v-else-if="providers">
            <p v-if="!providers.supported" class="text-muted-foreground">
              {{ t('chat.composer.picker.agentProvidersUnsupported') }}
            </p>
            <p v-else-if="!providers.providers.length" class="text-muted-foreground">
              {{ t('chat.composer.picker.agentProvidersEmpty') }}
            </p>
            <ul v-else class="grid gap-1.5">
              <li
                v-for="provider in providers.providers"
                :key="provider.provider_id"
                class="space-y-1 rounded-md border border-border/70 bg-muted/20 p-2"
              >
                <div class="flex flex-wrap items-center gap-1.5">
                  <span class="break-words font-medium text-foreground">{{ provider.provider_id }}</span>
                  <span
                    v-if="provider.required"
                    class="rounded-full border border-border/70 bg-background px-1.5 py-0.2 text-[10px] text-muted-foreground"
                  >
                    {{ t('chat.composer.picker.agentProviderRequired') }}
                  </span>
                </div>
                <div v-if="provider.supported.length" class="flex flex-wrap gap-1">
                  <span
                    v-for="apiType in provider.supported"
                    :key="apiType"
                    class="rounded bg-muted/70 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                  >
                    {{ apiType }}
                  </span>
                </div>
                <p class="break-all font-mono text-[11px] text-muted-foreground">
                  {{
                    provider.current
                      ? `${provider.current.api_type} · ${provider.current.base_url}`
                      : t('chat.composer.picker.agentProviderUnconfigured')
                  }}
                </p>
              </li>
            </ul>
          </template>
        </template>
      </div>
    </div>

    <!-- Compact bottom toolbar inside ACP sidebar -->
    <div class="flex items-center gap-1 px-2 py-1.5 text-[11px]">
      <button
        v-if="supportsDirectories"
        type="button"
        class="flex min-w-0 flex-1 items-center justify-center gap-1 rounded-md px-2 py-1 font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          activeTab === 'directories'
            ? 'bg-sidebar-accent text-foreground'
            : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground'
        "
        :title="t('chat.nativeAgent.directories')"
        :aria-expanded="activeTab === 'directories'"
        @click="toggleTab('directories')"
      >
        <RiFolder6Line class="h-3.5 w-3.5 shrink-0" />
        <span class="truncate">{{ t('chat.nativeAgent.sidebarDirectories') }}</span>
        <span
          v-if="directoriesList.length > 0"
          class="rounded-full bg-muted/80 px-1 text-[10px] tabular-nums text-foreground"
        >
          {{ directoriesList.length }}
        </span>
      </button>

      <button
        type="button"
        class="flex min-w-0 flex-1 items-center justify-center gap-1 rounded-md px-2 py-1 font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          activeTab === 'account'
            ? 'bg-sidebar-accent text-foreground'
            : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground'
        "
        :title="t('chat.composer.picker.agentAccount')"
        :aria-expanded="activeTab === 'account'"
        @click="toggleTab('account')"
      >
        <RiShieldUserLine class="h-3.5 w-3.5 shrink-0" />
        <span class="truncate">{{ t('chat.nativeAgent.sidebarAccount') }}</span>
        <span
          v-if="displayedAuth"
          class="h-1.5 w-1.5 shrink-0 rounded-full"
          :class="authDotClass"
          aria-hidden="true"
        />
      </button>

      <button
        type="button"
        class="flex min-w-0 flex-1 items-center justify-center gap-1 rounded-md px-2 py-1 font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          activeTab === 'providers'
            ? 'bg-sidebar-accent text-foreground'
            : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground'
        "
        :title="t('chat.composer.picker.agentProviders')"
        :aria-expanded="activeTab === 'providers'"
        @click="toggleTab('providers')"
      >
        <RiRouteLine class="h-3.5 w-3.5 shrink-0" />
        <span class="truncate">{{ t('chat.nativeAgent.sidebarProviders') }}</span>
        <span
          v-if="providers?.supported && routedProviderCount > 0"
          class="rounded-full bg-muted/80 px-1 text-[10px] tabular-nums text-foreground"
        >
          {{ routedProviderCount }}
        </span>
      </button>
    </div>
  </div>
</template>
