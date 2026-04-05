<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'

import LoginPage from './agena/pages/LoginPage.vue'
import { syncDesktopBackendTarget } from './lib/backend'
import { isDesktopRuntime } from './lib/desktopConfig'
import { useAuthStore } from './stores/auth'
import { useHealthStore } from './stores/health'

const auth = useAuthStore()
const health = useHealthStore()
const route = useRoute()

const booting = ref(true)

async function bootstrap() {
  booting.value = true
  try {
    if (isDesktopRuntime()) {
      await syncDesktopBackendTarget().catch(() => null)
    }
    await health.refresh().catch(() => {})
    await auth.refresh().catch(() => {})
  } finally {
    booting.value = false
  }
}

onMounted(() => {
  void bootstrap()
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
          <RouterLink
            to="/runtime"
            class="nav-link"
            :class="{ active: route.path.startsWith('/runtime') }"
          >
            Runtime
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
  </div>
</template>
