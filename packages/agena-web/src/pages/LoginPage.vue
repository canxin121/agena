<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useAuthStore } from '@/stores/auth'
import { useHealthStore } from '@/stores/health'
import { i18n, setAppLocale } from '@/i18n'
import type { AppLocale } from '@/i18n/locale'
import Button from '@/components/ui/Button.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { buildLoginLocalePickerOptions } from '@/pages/loginLocaleOptions'

const auth = useAuthStore()
const health = useHealthStore()
const { t } = useI18n()

const password = ref('')
const busy = ref(false)
const formError = ref<string | null>(null)

// The health probe below is a safety net: App.vue already probes health + auth
// on a timer and only mounts this page once the server is reachable, so this
// branch is rarely visible. Keeping it lets the login page degrade gracefully
// if the server goes away while the form is open.
let probeTimer: ReturnType<typeof setInterval> | null = null

const connecting = computed(() => health.data === null)

const visibleError = computed(() => formError.value || auth.lastError)

const uiLocale = computed<AppLocale>({
  get() {
    return i18n.global.locale.value as AppLocale
  },
  set(value) {
    setAppLocale(value)
  },
})

const localePickerOptions = computed(() => buildLoginLocalePickerOptions((key) => String(t(key))))

const canSubmit = computed(() => {
  if (busy.value) return false
  if (connecting.value) return false
  return true
})

async function refreshBootState() {
  try {
    await health.refresh().catch(() => {})
    if (health.data !== null) {
      await auth.refresh().catch(() => {})
    }
  } catch {
    // ignore
  }
}

function scheduleProbe() {
  if (!connecting.value) return
  if (probeTimer) return
  probeTimer = setInterval(() => {
    void refreshBootState()
  }, 2000)
}

function clearProbeTimer() {
  if (!probeTimer) return
  clearInterval(probeTimer)
  probeTimer = null
}

watch(
  () => connecting.value,
  (value) => {
    if (value) {
      scheduleProbe()
      return
    }
    clearProbeTimer()
  },
  { immediate: true },
)

onMounted(() => {
  if (health.data === null) {
    void refreshBootState()
  }
})

onBeforeUnmount(() => {
  clearProbeTimer()
})

async function submit() {
  if (!canSubmit.value) return
  const pwd = password.value
  if (!pwd.trim()) {
    formError.value = String(t('login.passwordRequired'))
    return
  }

  busy.value = true
  formError.value = null
  try {
    await auth.login(pwd)
    // auth.login refreshes state and populates lastError on failure. On success
    // needsLogin flips false and App.vue swaps to the main layout.
    password.value = ''
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="min-h-dvh bg-background px-4 py-6">
    <div class="mx-auto my-auto flex min-h-[calc(100dvh-3rem)] w-full max-w-[360px] flex-col justify-center space-y-6">
      <div class="flex flex-col items-center gap-4 text-center">
        <div
          class="flex h-16 w-16 items-center justify-center rounded-2xl bg-primary/10 ring-1 ring-inset ring-foreground/5"
        >
          <img src="/favicon.svg" alt="Agena" class="h-10 w-10 opacity-90" />
        </div>
        <div class="space-y-1">
          <h1 class="text-2xl font-semibold tracking-tight text-foreground">{{ t('login.title') }}</h1>
          <p class="text-sm text-muted-foreground">Enter your password to unlock Agena.</p>
        </div>
      </div>

      <div class="grid gap-4">
        <div class="grid gap-2">
          <label class="text-xs font-medium text-muted-foreground">{{ t('settings.appearance.language.label') }}</label>
          <div class="w-40 max-w-full">
            <OptionPicker
              v-model="uiLocale"
              :options="localePickerOptions"
              :title="String(t('settings.appearance.language.label'))"
              :search-placeholder="String(t('settings.appearance.language.label'))"
              :include-empty="false"
            />
          </div>
        </div>

        <div v-if="connecting" class="grid gap-3">
          <div class="flex items-center justify-center gap-3 rounded-lg border border-border bg-muted/10 px-4 py-6">
            <div class="h-4 w-4 animate-spin rounded-full border-2 border-primary/30 border-t-primary" />
            <div class="text-sm text-muted-foreground">Connecting to server...</div>
          </div>
        </div>

        <template v-else>
          <div class="grid gap-2">
            <Input
              id="password"
              v-model="password"
              type="password"
              :placeholder="String(t('login.passwordPlaceholder'))"
              autocomplete="current-password"
              @keydown.enter="submit"
              :disabled="busy"
              autofocus
              class="h-11 bg-muted/30 text-center text-lg placeholder:text-muted-foreground/50"
            />
          </div>
          <Button
            :disabled="!canSubmit"
            @click="submit"
            class="h-11 w-full text-base font-medium shadow-lg shadow-primary/20 transition-all hover:shadow-primary/30"
            size="lg"
          >
            {{ busy ? t('common.connecting') : t('common.unlock') }}
          </Button>
        </template>
      </div>

      <div
        v-if="visibleError"
        class="animate-in fade-in slide-in-from-top-2 rounded-lg bg-destructive/10 p-3 text-sm font-medium text-destructive ring-1 ring-inset ring-destructive/20"
      >
        <div class="break-words text-center">{{ visibleError }}</div>
      </div>
    </div>
  </div>
</template>
