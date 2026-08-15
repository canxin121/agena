<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiLoader4Line } from '@remixicon/vue'

import { apiJson } from '@/lib/api'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import Button from '@/components/ui/Button.vue'
import SessionHubRow, { type HubRowKind, type SessionResource } from '@/components/hub/SessionHubRow.vue'

/** Response of GET /api/v1/sessions/overview. */
interface SessionOverview {
  attention: SessionResource[]
  running: SessionResource[]
  recent: SessionResource[]
  generated_at: string
}

type HubSection = {
  id: HubRowKind
  title: string
  sessions: SessionResource[]
  emptyKey: string
  dotClass: string
}

const router = useRouter()
const chat = useChatStore()
const toasts = useToastsStore()
const { t } = useI18n()

const loading = ref(true)
const loadError = ref<string | null>(null)
const overview = ref<SessionOverview | null>(null)
const creating = ref(false)

// Bumped every minute so relative timestamps stay fresh on a long-lived page.
const nowTick = ref(Date.now())
let tickTimer: ReturnType<typeof setInterval> | null = null

const attentionSessions = computed(() => overview.value?.attention ?? [])
const runningSessions = computed(() => overview.value?.running ?? [])
const recentSessions = computed(() => overview.value?.recent ?? [])

const sections = computed<HubSection[]>(() => [
  {
    id: 'attention',
    title: String(t('hub.attention')),
    sessions: attentionSessions.value,
    emptyKey: 'hub.emptyAttention',
    dotClass: 'bg-amber-500',
  },
  {
    id: 'running',
    title: String(t('hub.running')),
    sessions: runningSessions.value,
    emptyKey: 'hub.emptyRunning',
    dotClass: 'bg-sky-500',
  },
  {
    id: 'recent',
    title: String(t('hub.recent')),
    sessions: recentSessions.value,
    emptyKey: 'hub.emptyRecent',
    dotClass: 'bg-muted-foreground',
  },
])

async function loadOverview() {
  loading.value = true
  loadError.value = null
  try {
    overview.value = await apiJson<SessionOverview>('/api/v1/sessions/overview?recent_limit=50')
  } catch (err) {
    loadError.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function openSession(id: number) {
  try {
    await chat.selectSession(String(id))
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
    return
  }
  router.push('/chat')
}

async function newSession() {
  if (creating.value) return
  creating.value = true
  try {
    const created = await chat.createSession()
    if (created?.id) {
      await chat.selectSession(String(created.id))
    }
    router.push('/chat')
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    creating.value = false
  }
}

onMounted(() => {
  void loadOverview()
  tickTimer = setInterval(() => {
    nowTick.value = Date.now()
  }, 60_000)
})

onBeforeUnmount(() => {
  if (tickTimer) {
    clearInterval(tickTimer)
    tickTimer = null
  }
})
</script>

<template>
  <div class="h-full min-h-0 overflow-y-auto bg-background">
    <div class="mx-auto w-full max-w-3xl px-4 py-6 sm:px-6">
      <header class="mb-6 flex items-center justify-between gap-3">
        <h1 class="min-w-0 truncate text-xl font-semibold text-foreground">{{ t('hub.title') }}</h1>
        <Button variant="default" size="sm" :disabled="creating" @click="newSession">
          <RiLoader4Line v-if="creating" class="mr-1.5 h-4 w-4 animate-spin" />
          <RiAddLine v-else class="mr-1.5 h-4 w-4" />
          {{ t('hub.newSession') }}
        </Button>
      </header>

      <div v-if="loading" class="py-12 text-center text-sm text-muted-foreground">{{ t('hub.loading') }}</div>

      <div v-else-if="loadError" class="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-8 text-center">
        <p class="text-sm text-destructive">{{ loadError }}</p>
        <Button variant="outline" size="sm" class="mt-3" @click="loadOverview">{{ t('common.retry') }}</Button>
      </div>

      <template v-else>
        <section v-for="sec in sections" :key="sec.id" class="mb-6">
          <div class="mb-2 flex items-center gap-2">
            <span class="h-2 w-2 shrink-0 rounded-full" :class="sec.dotClass" aria-hidden="true" />
            <h2 class="text-sm font-semibold text-foreground">{{ sec.title }}</h2>
            <span
              v-if="sec.sessions.length"
              class="rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground"
            >
              {{ sec.sessions.length }}
            </span>
          </div>

          <div v-if="sec.sessions.length" class="space-y-2">
            <SessionHubRow
              v-for="s in sec.sessions"
              :key="s.id"
              :session="s"
              :kind="sec.id"
              :now="nowTick"
              @open="openSession(s.id)"
            />
          </div>

          <div
            v-else
            class="rounded-lg border border-dashed border-border/60 px-4 py-8 text-center text-sm text-muted-foreground"
          >
            {{ t(sec.emptyKey) }}
          </div>
        </section>
      </template>
    </div>
  </div>
</template>
