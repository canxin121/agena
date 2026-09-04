<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch, watchEffect } from 'vue'
import { useI18n } from 'vue-i18n'

import { useAuthStore } from './stores/auth'
import { useHealthStore } from './stores/health'
import { useSettingsStore } from './stores/settings'

import { applyAppearanceSettingsToDom } from './lib/appearance'
import { useDeviceRuntime } from './app/runtime/useDeviceRuntime'

import LoginPage from './pages/LoginPage.vue'
import MainLayout from './layout/MainLayout.vue'
import AppConfirmHost from './components/AppConfirmHost.vue'
import AppTextPromptHost from './components/AppTextPromptHost.vue'
import ToastHost from './components/ToastHost.vue'

const auth = useAuthStore()
const health = useHealthStore()
const settings = useSettingsStore()
const { t } = useI18n()
useDeviceRuntime()

const backendReady = computed(() => health.data !== null)
const showLogin = computed(() => !showLoading.value && (auth.needsLogin || !backendReady.value))
// Do not mount the previous page while /auth/session is still being checked.
// Its initial protected requests would otherwise emit 401s and race the login
// page into existence before the auth state has settled.
const showLoading = computed(() => health.data === null || !auth.checked)

let probeTimer: ReturnType<typeof setInterval> | null = null
let probeBusy = false
let systemThemeMedia: MediaQueryList | null = null

function handleSystemThemeChange() {
  applyAppearanceSettingsToDom(settings.data)
}

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
  if (typeof window.matchMedia === 'function') {
    systemThemeMedia = window.matchMedia('(prefers-color-scheme: light)')
    systemThemeMedia.addEventListener?.('change', handleSystemThemeChange)
  }
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
  systemThemeMedia?.removeEventListener?.('change', handleSystemThemeChange)
  systemThemeMedia = null
})

watchEffect(() => {
  // Apply theme + typography immediately when settings change.
  applyAppearanceSettingsToDom(settings.data)
})
</script>

<template>
  <div class="app-root">
    <ToastHost />
    <AppConfirmHost />
    <AppTextPromptHost />
    <div
      v-if="showLoading"
      role="status"
      aria-live="polite"
      :aria-label="String(t('common.loading'))"
      class="flex h-full w-full items-center justify-center bg-background"
    >
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
