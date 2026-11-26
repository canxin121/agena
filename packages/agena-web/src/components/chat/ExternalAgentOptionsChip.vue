<script setup lang="ts">
import { sessionApiPath } from '@/lib/acpSession'
import { RiRefreshLine, RiStackLine } from '@remixicon/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import SessionChip from './SessionChip.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import type { ExternalAgentAuthStatus, ExternalAgentOptions } from '@/types/chat'

const props = defineProps<{ sessionId: string; label: string; liveOptions?: ExternalAgentOptions }>()
const { t } = useI18n()
const open = ref(false)
const loading = ref(false)
const error = ref('')
const options = ref<ExternalAgentOptions>()
type AgentAuth = Partial<ExternalAgentAuthStatus> & {
  methods: { id: string; name: string; description?: string; type: 'agent' | 'terminal' }[]
  logout_supported: boolean
}
type AuthAction = { action: 'inspect' | 'logout' } | { action: 'login'; method_id: string }
const authOpen = ref(false)
const authBusy = ref(false)
const authError = ref('')
const auth = ref<AgentAuth>()
const liveAuth = ref<ExternalAgentAuthStatus | null>(null)
const displayedAuth = computed(() => liveAuth.value ?? auth.value)
const authPath = `${sessionApiPath(props.sessionId)}/agent-auth`
const authStateKey = computed(() => ({
  authenticated: 'agentSignedIn',
  unauthenticated: 'agentSignInRequired',
  not_required: 'agentSignInNotRequired',
  unknown: 'agentStatusUnavailable',
}[displayedAuth.value?.state ?? 'unknown']))
const title = computed(() => `${props.label} · ${t('chat.composer.picker.optionsTitle')}`)
const rows = computed(() => {
  if (!options.value) return []
  const { selection, options: available } = options.value
  const advertised = available.map((option) => ({ ...option, selected: selection[option.kind] ?? '' }))
  const missing = Object.entries(selection).flatMap(([kind, selected]) => {
    if (!selected || advertised.some((option) => option.kind === kind)) return []
    return [{
      id: kind,
      kind,
      selected,
      name: kind.startsWith('config:') ? kind.slice(7) : kind,
      description: undefined,
      current_value: '',
      choices: [],
    }]
  })
  return [...advertised, ...missing]
})
watch(() => props.liveOptions, (live) => {
  if (live) options.value = live
}, { immediate: true })
const chipLabel = computed(() => {
  const model = rows.value.find((row) => row.kind === 'model')
  const value = model?.current_value || model?.selected
  if (!model || !value) return props.label
  const name = model.choices.find((choice) => choice.value === value)?.name || value
  return `${props.label} · ${name}`
})

async function load(change?: { kind: string; value: string | null }) {
  if (loading.value) return
  loading.value = true
  error.value = ''
  try {
    options.value = await apiJson<ExternalAgentOptions>(`${sessionApiPath(props.sessionId)}/agent-options`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(change ? { change } : {}),
    })
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

function setOpen(next: boolean) {
  open.value = next
  if (next) void load()
}

function select(kind: string, event: Event) {
  const input = event.currentTarget as HTMLSelectElement
  const value = input.value
  input.value = options.value?.selection[kind] ?? ''
  void load({ kind, value: value || null })
}

async function runAuth(action: AuthAction = { action: 'inspect' }) {
  if (authBusy.value) return
  authBusy.value = true
  authError.value = ''
  try {
    auth.value = await apiJson<AgentAuth>(authPath, {
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

function toggleAuth() {
  authOpen.value = !authOpen.value
  if (authOpen.value) void runAuth()
}

watch([open, authOpen], ([panel, account], _previous, onCleanup) => {
  if (!panel || !account) {
    liveAuth.value = null
    return
  }
  const controller = new AbortController()
  let refreshing = false
  async function refresh() {
    if (refreshing || controller.signal.aborted) return
    refreshing = true
    try {
      const status = await apiJson<ExternalAgentAuthStatus | null>(authPath, { signal: controller.signal })
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
})
</script>

<template>
  <SessionChip
    :title="title"
    :label="chipLabel"
    :open="open"
    :busy="loading"
    panel-class="w-[min(92vw,22rem)]"
    @update:open="setOpen"
  >
    <template #icon>
      <RiStackLine class="h-3 w-3" />
    </template>
    <template #actions>
      <IconButton :aria-label="t('common.refresh')" :tooltip="t('common.refresh')" size="sm" :disabled="loading" @click="load()">
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <div class="grid gap-2.5 py-1 text-xs" :aria-busy="loading">
      <p v-if="error" role="alert" class="whitespace-pre-wrap text-destructive">{{ error }}</p>
      <p v-if="!options && loading" role="status" class="text-muted-foreground">{{ t('chat.composer.status.loading') }}</p>
      <p v-if="options && !rows.length && !loading" class="text-muted-foreground">
        {{ t('chat.composer.picker.emptyOptions') }}
      </p>
      <label v-for="row in rows" :key="row.kind" class="grid min-w-0 gap-1">
        <span class="text-muted-foreground">{{ row.name }}</span>
        <span v-if="row.description" class="text-muted-foreground">{{ row.description }}</span>
        <select
          :value="row.selected"
          :disabled="loading"
          class="min-h-8 w-full rounded border border-input bg-background px-2 text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
          @change="select(row.kind, $event)"
        >
          <option value="">
            {{ t('chat.composer.picker.agentSettings') }}{{ row.current_value ? ` · ${row.choices.find((choice) => choice.value === row.current_value)?.name || row.current_value}` : '' }}
          </option>
          <option v-if="row.selected && !row.choices.some((choice) => choice.value === row.selected)" :value="row.selected" disabled>
            {{ row.selected }}
          </option>
          <option v-for="choice in row.choices" :key="choice.value" :value="choice.value" :title="choice.description">
            {{ choice.name }}
          </option>
        </select>
      </label>
      <p v-if="rows.length" class="text-muted-foreground">{{ t('chat.composer.picker.agentOptionNextTurn') }}</p>
      <div class="grid gap-2 border-t border-border pt-2">
        <Button size="sm" variant="outline" class="justify-self-start" :aria-expanded="authOpen" @click="toggleAuth">
          {{ t('chat.composer.picker.agentAccount') }}
        </Button>
        <div v-if="authOpen" class="grid gap-2" :aria-busy="authBusy">
          <p v-if="authError" role="alert" class="whitespace-pre-wrap text-destructive">{{ authError }}</p>
          <p v-else-if="authBusy" role="status" class="text-muted-foreground">{{ t('chat.composer.status.loading') }}</p>
          <template v-if="displayedAuth">
            <p role="status" class="text-muted-foreground">
              <span v-if="liveAuth">{{ t('chat.composer.picker.agentConnectionStatus') }} · </span>
              {{ t(`chat.composer.picker.${authStateKey}`) }}
            </p>
            <div v-for="(account, index) in displayedAuth.accounts" :key="index" class="break-words">
              <p>{{ [account.label, account.name, account.email, account.organization].filter(Boolean).join(' · ') }}</p>
              <p v-if="account.description" class="text-muted-foreground">{{ account.description }}</p>
            </div>
          </template>
          <template v-if="auth">
            <div class="flex flex-wrap gap-1.5">
              <Button
                v-for="method in auth.methods"
                :key="method.id"
                size="sm"
                variant="outline"
                :disabled="authBusy || method.type !== 'agent'"
                :title="method.description"
                @click="runAuth({ action: 'login', method_id: method.id })"
              >{{ method.name }}</Button>
              <Button
                v-if="auth.logout_supported"
                size="sm"
                variant="outline"
                :disabled="authBusy"
                @click="runAuth({ action: 'logout' })"
              >{{ t('chat.composer.picker.agentSignOut') }}</Button>
            </div>
            <p class="text-muted-foreground">
              {{ t('chat.composer.picker.agentTerminalSignIn') }}
              <code>agena sessions agent-auth {{ sessionId }} --terminal</code>
            </p>
          </template>
        </div>
      </div>
    </div>
  </SessionChip>
</template>
