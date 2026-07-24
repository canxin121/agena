<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'

import LoginPage from './agena/pages/LoginPage.vue'
import { dispatchAuthCallback } from './agena/lib/authCallbackRegistry'
import { buildPluginCommandPayload, createCommandPalette, type CommandItem } from './agena/lib/commandPalette'
import { registeredLocalCommands, setGlobalCommandPaletteOpenHandler } from './agena/lib/commandPaletteRegistry'
import {
  fetchRuntimeStatus,
  runPluginUiAction,
  type PluginStudioCommand,
  type RuntimeStatus,
  type RuntimeSkill,
} from './agena/lib/agenaApi'
import { isPluginUiToolInvokeResponse, resolvePluginCommandOutput } from './agena/lib/pluginUiActionRuntime'
import { usePluginToolRegistryRuntimeSync } from './agena/lib/usePluginToolRegistryRuntimeSync'
import { sectionBasePaths, sectionNavItems } from './agena/pages/runtimePageStateModel'
import { useAuthStore } from './stores/auth'
import { useHealthStore } from './stores/health'

const auth = useAuthStore()
const health = useHealthStore()
const route = useRoute()
const router = useRouter()

const booting = ref(true)
const runtimeSnapshot = ref<RuntimeStatus | null>(null)
const runtimeSkills = ref<RuntimeSkill[]>([])
const runtimeCommands = ref<RuntimeSkill[]>([])
const pluginCommands = ref<PluginStudioCommand[]>([])
const commandPalette = createCommandPalette({
  router,
  runtimeSkills: computed(() => runtimeSkills.value),
  runtimeCommands: computed(() => runtimeCommands.value),
  pluginCommands: computed(() => pluginCommands.value),
  localCommands: registeredLocalCommands,
  onSelectRuntimeEntry: async ({ item }) => {
    await router.push({ path: '/chat', query: { slash: `/${item.name}` } })
    commandPalette.closePalette()
  },
  onRunPluginAction: async ({ command, context }) => {
    const action = command.action
    if (action.kind === 'open_route') {
      await router.push(action.route)
      return
    }
    if (action.kind === 'open_url') {
      if (typeof window !== 'undefined') window.open(action.url, '_blank', 'noopener,noreferrer')
      return
    }
    if (action.kind === 'submit_prompt') {
      await router.push({ path: '/chat', query: { prompt: action.prompt } })
      return
    }
    if (action.kind === 'invoke_tool') {
      const response = await runPluginUiAction({
        pluginId: command.plugin_id,
        actionId: command.id,
        payload: buildPluginCommandPayload(command, context),
      })
      if (action.submit_output_as_prompt && isPluginUiToolInvokeResponse(response.result)) {
        await router.push({ path: '/chat', query: { prompt: response.result.output_text } })
      }
      return
    }
    if (action.kind === 'invoke_command') {
      const response = await runPluginUiAction({
        pluginId: command.plugin_id,
        actionId: command.id,
        payload: buildPluginCommandPayload(command, context),
      })
      await applyResolvedPluginCommandEffect(
        await resolvePluginCommandOutput({
          pluginId: command.plugin_id,
          result: response.result,
        }),
      )
    }
  },
})

async function applyResolvedPluginCommandEffect(effect: Awaited<ReturnType<typeof resolvePluginCommandOutput>>) {
  if (effect.kind === 'submit_prompt') {
    await router.push({ path: '/chat', query: { prompt: effect.prompt } })
    return
  }
  if (effect.kind === 'open_route') {
    await router.push(effect.route)
    return
  }
  if (effect.kind === 'open_url') {
    if (typeof window !== 'undefined') window.open(effect.url, '_blank', 'noopener,noreferrer')
    return
  }
}

function runPaletteItem(item: CommandItem) {
  void item.run()
  commandPalette.closePalette()
}

function applyRuntimeSnapshot(status: RuntimeStatus | null) {
  runtimeSnapshot.value = status
  runtimeSkills.value = status?.operator.skills.skills ?? []
  runtimeCommands.value = status?.operator.skills.commands ?? []
  pluginCommands.value = status?.operator.ui?.catalog.studio.commands ?? []
}

const pluginRegistryRuntimeSync = usePluginToolRegistryRuntimeSync(
  {
    runtime: runtimeSnapshot,
  },
  undefined,
  {
    registerComponentLifecycle: false,
    onRuntimeRefreshed: (status) => {
      applyRuntimeSnapshot(status)
    },
    onError: (error) => {
      console.warn('plugin tool registry stream failed', error)
    },
  },
)

function handleGlobalKeydown(event: KeyboardEvent) {
  const key = String(event.key || '').toLowerCase()
  const isPaletteShortcut =
    key === 'p' && ((event.metaKey && !event.ctrlKey) || (!event.metaKey && event.ctrlKey)) && event.shiftKey
  const isEscape = key === 'escape'
  const isEnter = key === 'enter'
  const isArrowDown = key === 'arrowdown'
  const isArrowUp = key === 'arrowup'

  if (isPaletteShortcut) {
    event.preventDefault()
    commandPalette.togglePalette()
    return
  }

  if (!commandPalette.open.value) return

  if (isEscape) {
    event.preventDefault()
    commandPalette.closePalette()
    return
  }

  if (isEnter) {
    event.preventDefault()
    void commandPalette.runHighlighted()
    return
  }

  if (isArrowDown) {
    event.preventDefault()
    commandPalette.moveHighlight(1)
    return
  }

  if (isArrowUp) {
    event.preventDefault()
    commandPalette.moveHighlight(-1)
  }
}

function handleWindowMessage(event: MessageEvent) {
  const data = event.data
  if (!data || typeof data !== 'object' || Array.isArray(data)) return
  const payload = data as Record<string, unknown>
  if (payload.type !== 'agena-auth-callback') return
  void dispatchAuthCallback({
    code: typeof payload.code === 'string' ? payload.code : '',
    state: typeof payload.state === 'string' ? payload.state : '',
    error: typeof payload.error === 'string' ? payload.error : '',
  })
}

async function bootstrap() {
  booting.value = true
  try {
    await Promise.all([
      health.refresh().catch(() => {}),
      auth.refresh().catch(() => {}),
      fetchRuntimeStatus()
        .then((status) => {
          applyRuntimeSnapshot(status)
        })
        .catch(() => {
          applyRuntimeSnapshot(null)
        }),
    ])
  } finally {
    booting.value = false
  }
}

onMounted(() => {
  setGlobalCommandPaletteOpenHandler(commandPalette.openPalette)
  void bootstrap()
  if (typeof window !== 'undefined') {
    window.addEventListener('keydown', handleGlobalKeydown)
    window.addEventListener('message', handleWindowMessage)
  }
})

onBeforeUnmount(() => {
  setGlobalCommandPaletteOpenHandler(null)
  pluginRegistryRuntimeSync.stop()
  if (typeof window !== 'undefined') {
    window.removeEventListener('keydown', handleGlobalKeydown)
    window.removeEventListener('message', handleWindowMessage)
  }
})

const backendReady = computed(() => health.data !== null)
const showLogin = computed(() => backendReady.value && auth.needsLogin)
const activeModeLabel = computed(() => {
  const value = String(health.data?.activeMode || '').trim()
  return value || 'default'
})
const pluginRegistrySyncEnabled = computed(() => backendReady.value && !showLogin.value)

watch(
  pluginRegistrySyncEnabled,
  (enabled) => {
    if (enabled) {
      pluginRegistryRuntimeSync.start()
      void pluginRegistryRuntimeSync.refreshRuntime()
    } else {
      pluginRegistryRuntimeSync.stop()
      applyRuntimeSnapshot(null)
    }
  },
  { immediate: true },
)
</script>

<template>
  <div class="shell">
    <div v-if="booting" class="boot-screen">
      <div class="panel">
        <div class="eyebrow">Agena</div>
        <h1>Starting runtime</h1>
        <p>Probing the local backend and checking UI authentication.</p>
      </div>
    </div>

    <div v-else-if="!backendReady" class="boot-screen">
      <div class="panel">
        <div class="eyebrow">Agena</div>
        <h1>Backend unavailable</h1>
        <p>{{ health.error || 'The backend did not answer /health.' }}</p>
        <button class="button primary" @click="bootstrap">Retry</button>
      </div>
    </div>

    <LoginPage v-else-if="showLogin" />

    <div v-else class="app-frame">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark">A</div>
          <div>
            <div class="brand-title">Agena</div>
            <div class="brand-subtitle">Gen {{ health.data?.generation }} · mode {{ activeModeLabel }}</div>
          </div>
        </div>

        <nav class="nav">
          <RouterLink to="/chat" class="nav-link" :class="{ active: route.path.startsWith('/chat') }">
            Chat
          </RouterLink>
          <RouterLink to="/workspace" class="nav-link" :class="{ active: route.path.startsWith('/workspace') }">
            Workspace
          </RouterLink>
          <RouterLink to="/usage" class="nav-link" :class="{ active: route.path.startsWith('/usage') }">
            Usage
          </RouterLink>
          <RouterLink
            v-for="item in sectionNavItems"
            :key="item.section"
            :to="sectionBasePaths[item.section]"
            class="nav-link"
            :class="{ active: route.path.startsWith(sectionBasePaths[item.section]) }"
          >
            {{ item.label }}
          </RouterLink>
        </nav>

        <div class="sidebar-meta">
          <div class="meta-label">Workspace Root</div>
          <div class="meta-value">{{ health.data?.workspaceRoot }}</div>
          <div class="meta-label">Config</div>
          <div class="meta-value">{{ health.data?.configPath }}</div>
        </div>
      </aside>

      <main class="main-content">
        <RouterView />
      </main>
    </div>

    <div v-if="commandPalette.open.value" class="palette-backdrop" @click="commandPalette.closePalette()">
      <section class="palette" @click.stop>
        <div class="field">
          <label class="label" for="global-command-palette">Command Palette</label>
          <input
            id="global-command-palette"
            v-model="commandPalette.query.value"
            class="input mono"
            placeholder="Search commands, pages, skills, and runtime actions"
            autofocus
          />
        </div>
        <div v-if="commandPalette.filteredItems.value.length" class="list" style="margin-top: 12px">
          <button
            v-for="(item, index) in commandPalette.filteredItems.value"
            :key="item.id"
            class="list-item palette-item"
            :class="{ active: index === commandPalette.highlightedIndex.value }"
            @click="runPaletteItem(item)"
          >
            <div>
              <strong>{{ item.title }}</strong>
              <div class="muted">{{ item.description }}</div>
              <div v-if="item.sourceLabel" class="muted mono">source={{ item.sourceLabel }}</div>
            </div>
            <div class="stack" style="justify-items: end">
              <span class="badge">{{ item.category }}</span>
              <span v-if="item.usage || item.slash" class="muted mono">{{ item.usage || item.slash }}</span>
            </div>
          </button>
        </div>
        <p v-else class="muted" style="margin-top: 12px">No commands matched the current query.</p>
      </section>
    </div>
  </div>
</template>
