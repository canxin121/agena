<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'

import LoginPage from './agena/pages/LoginPage.vue'
import { createCommandPalette } from './agena/lib/commandPalette'
import { registeredLocalCommands, setGlobalCommandPaletteOpenHandler } from './agena/lib/commandPaletteRegistry'
import { fetchRuntimeStatus } from './agena/lib/agenaApi'
import { sectionBasePaths, sectionNavItems } from './agena/pages/runtimePageStateModel'
import { syncDesktopBackendTarget } from './lib/backend'
import { isDesktopRuntime } from './lib/desktopConfig'
import { useAuthStore } from './stores/auth'
import { useHealthStore } from './stores/health'

const auth = useAuthStore()
const health = useHealthStore()
const route = useRoute()
const router = useRouter()

const booting = ref(true)
const runtimeSkills = ref([])
const runtimeCommands = ref([])
const commandPalette = createCommandPalette({
  router,
  runtimeSkills: computed(() => runtimeSkills.value),
  runtimeCommands: computed(() => runtimeCommands.value),
  localCommands: registeredLocalCommands,
  onSelectRuntimeEntry: async ({ item }) => {
    await router.push({ path: '/chat', query: { slash: `/${item.name}` } })
    commandPalette.closePalette()
  },
})

function handleGlobalKeydown(event: KeyboardEvent) {
  const key = String(event.key || '').toLowerCase()
  const isPaletteShortcut = key === 'p' && ((event.metaKey && !event.ctrlKey) || (!event.metaKey && event.ctrlKey)) && event.shiftKey
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

async function bootstrap() {
  booting.value = true
  try {
    if (isDesktopRuntime()) {
      await syncDesktopBackendTarget().catch(() => null)
    }
    await Promise.all([
      health.refresh().catch(() => {}),
      auth.refresh().catch(() => {}),
      fetchRuntimeStatus()
        .then((status) => {
          runtimeSkills.value = status.operator.skills.skills
          runtimeCommands.value = status.operator.skills.commands
        })
        .catch(() => {
          runtimeSkills.value = []
          runtimeCommands.value = []
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
  }
})

onBeforeUnmount(() => {
  setGlobalCommandPaletteOpenHandler(null)
  if (typeof window !== 'undefined') {
    window.removeEventListener('keydown', handleGlobalKeydown)
  }
})

const backendReady = computed(() => health.data !== null)
const showLogin = computed(() => backendReady.value && auth.needsLogin)
const activeModeLabel = computed(() => {
  const value = String(health.data?.activeMode || '').trim()
  return value || 'default'
})
</script>

<template>
  <div class="shell">
    <div v-if="booting" class="boot-screen">
      <div class="panel">
        <div class="eyebrow">Agena Studio</div>
        <h1>Starting runtime</h1>
        <p>Probing the local backend and checking UI authentication.</p>
      </div>
    </div>

    <div v-else-if="!backendReady" class="boot-screen">
      <div class="panel">
        <div class="eyebrow">Agena Studio</div>
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
            <div class="brand-title">Agena Studio</div>
            <div class="brand-subtitle">
              Gen {{ health.data?.generation }} · mode {{ activeModeLabel }}
            </div>
          </div>
        </div>

        <nav class="nav">
          <RouterLink to="/chat" class="nav-link" :class="{ active: route.path.startsWith('/chat') }">
            Chat
          </RouterLink>
          <RouterLink to="/workspace" class="nav-link" :class="{ active: route.path.startsWith('/workspace') }">
            Workspace
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
            @click="void item.run(); commandPalette.closePalette()"
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
