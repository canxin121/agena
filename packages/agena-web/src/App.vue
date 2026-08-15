<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch, watchEffect } from 'vue'

import { useAuthStore } from './stores/auth'
import { useHealthStore } from './stores/health'
import { useSettingsStore } from './stores/settings'

import { applyAppearanceSettingsToDom } from './lib/appearance'

import LoginPage from './pages/LoginPage.vue'
import MainLayout from './layout/MainLayout.vue'
import ToastHost from './components/ToastHost.vue'

const auth = useAuthStore()
const health = useHealthStore()
const settings = useSettingsStore()

const backendReady = computed(() => health.data !== null)
const showLogin = computed(() => !showLoading.value && (auth.needsLogin || !backendReady.value))
const showLoading = computed(() => health.data === null)

let probeTimer: ReturnType<typeof setInterval> | null = null
let probeBusy = false

async function refreshBootState() {
  if (probeBusy) return
  probeBusy = true
  try {
    await health.refresh().catch(() => {})
    if (health.data !== null) {
      await auth.refresh().catch(() => {})
    }
  } finally {
    probeBusy = false
  }
}

function clearProbeTimer() {
  if (!probeTimer) return
  clearInterval(probeTimer)
  probeTimer = null
}

function scheduleProbe() {
  if (!showLoading.value) return
  if (probeTimer) return
  probeTimer = setInterval(() => {
    void refreshBootState()
  }, 2000)
}

onMounted(() => {
  void refreshBootState()
})

watch(
  () => showLoading.value,
  (loading) => {
    if (loading) {
      scheduleProbe()
      return
    }
    clearProbeTimer()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  clearProbeTimer()
})

watchEffect(() => {
  // Apply theme + typography immediately when settings change.
  applyAppearanceSettingsToDom(settings.data)
})
</script>

<template>
  <div class="app-root">
    <ToastHost />
    <div v-if="showLoading" class="flex h-full w-full items-center justify-center bg-background">
      <div class="h-6 w-6 animate-spin rounded-full border-2 border-primary/30 border-t-primary" />
    </div>
    <LoginPage v-else-if="showLogin" />
    <MainLayout v-else />
  </div>
</template>

<style scoped>
.app-root {
  height: 100%;
}
</style>
