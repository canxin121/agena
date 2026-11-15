<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiArrowRightSLine, RiFolderLine, RiGlobalLine, RiToolsLine } from '@remixicon/vue'
import { apiJson } from '@/lib/api'
import { useChatStore } from '@/stores/chat'
import { lastSessionId } from '@/lib/lastSession'
import { settingsText as st } from '@/i18n/settingsText'
import Button from '@/components/ui/Button.vue'
import SettingsCard from './SettingsCard.vue'

import {
  parsePermissionConfig,
  permissionConfigErrorMessage,
  type PermissionConfig,
  type PermissionMode,
  type PathPermissionMode,
} from '@/lib/permissionConfig'
type SettingsResponse = { value?: unknown }
type SessionStateResponse = {
  execution?: {
    effective_permission?: unknown
    context?: { effective_permission?: unknown }
    execution?: { effective_permission?: unknown }
  }
}

const props = defineProps<{ sessionId?: string }>()
const route = useRoute()
const router = useRouter()
const chat = useChatStore()
const busy = ref(false)
const error = ref('')
const permission = ref<PermissionConfig>({})
const sessionId = computed(() => {
  const id = Number(String(props.sessionId || '').trim() || lastSessionId.value || chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
let controller: AbortController | undefined

function asPolicy(value: unknown): PermissionConfig {
  return value == null ? {} : parsePermissionConfig(value)
}

async function refresh() {
  controller?.abort()
  const current = new AbortController()
  controller = current
  busy.value = true
  error.value = ''
  try {
    let effective: PermissionConfig = {}
    if (sessionId.value) {
      const result = await apiJson<SessionStateResponse>(`/api/v1/sessions/${sessionId.value}/state`, {
        signal: current.signal,
      })
      const execution = result.execution
      effective = asPolicy(
        execution?.execution?.effective_permission ||
          execution?.effective_permission ||
          execution?.context?.effective_permission,
      )
    }
    if (!Object.keys(effective).length) {
      const result = await apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission', {
        signal: current.signal,
      })
      effective = asPolicy(result.value)
    }
    if (controller !== current) return
    permission.value = effective
  } catch (reason) {
    if (controller !== current || current.signal.aborted) return
    error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    if (controller === current) busy.value = false
  }
}

const cards = computed(() => {
  const policy = permission.value
  return [
    {
      id: 'filesystem',
      title: st('File access'),
      icon: RiFolderLine,
      count: Object.keys(policy.path?.rules || {}).length,
      rows: [
        { label: st('Workspace files'), mode: policy.path?.workspace },
        { label: st('External files'), mode: policy.path?.external },
      ],
    },
    {
      id: 'network',
      title: st('Network access'),
      icon: RiGlobalLine,
      count: (policy.network?.egress?.allow?.length || 0) + (policy.network?.egress?.deny?.length || 0),
      rows: [
        { label: st('Outbound default'), mode: policy.network?.egress?.default ?? ('deny' as const) },
        { label: st('Inbound default'), mode: policy.network?.ingress?.default ?? ('deny' as const) },
        { label: st('Host loopback'), mode: policy.network?.ingress?.hostLoopback ?? ('deny' as const) },
      ],
    },
    {
      id: 'tools',
      title: st('Tool execution & approvals'),
      icon: RiToolsLine,
      count: Object.keys(policy.tools?.names || {}).length + Object.keys(policy.tools?.rules || {}).length,
      rows: [{ label: st('Default'), mode: policy.tools?.default }],
    },
  ]
})

function modeLabel(mode: PermissionMode | PathPermissionMode | undefined): string {
  if (mode === 'readonly') return st('Read only')
  if (mode === 'readwrite') return st('Read and write')
  if (mode === 'allow') return st('Allow')
  if (mode === 'auto') return st('Auto')
  if (mode === 'ask') return st('Ask')
  if (mode === 'deny') return st('Deny')
  return st('Not configured')
}

function openDetails(view: string) {
  void router.push({
    path: '/settings/permissions',
    query: { ...route.query, view, scope: 'effective' },
    hash: route.hash,
  })
}

watch(sessionId, () => void refresh())
onMounted(() => void refresh())
onBeforeUnmount(() => controller?.abort())
defineExpose({ refresh })
</script>

<template>
  <section class="space-y-4" :aria-busy="busy">
    <p class="text-sm text-muted-foreground">{{ $st('Read-only merged policy.') }}</p>
    <p class="text-xs text-muted-foreground">{{ $st('Specific rules can override these defaults.') }}</p>
    <p v-if="error" role="alert" class="rounded-lg border border-destructive/30 p-3 text-sm text-destructive">
      {{ error }}
      <Button variant="outline" size="sm" class="ml-2" @click="refresh">{{ $st('Retry') }}</Button>
    </p>
    <p v-else-if="busy" role="status" class="text-sm text-muted-foreground">{{ $st('Loading…') }}</p>
    <div v-if="!error" class="grid gap-4 xl:grid-cols-2">
      <SettingsCard v-for="card in cards" :key="card.id" :title="card.title" :icon="card.icon" :count="card.count">
        <template #actions>
          <Button variant="outline" size="sm" @click="openDetails(card.id)">
            {{ $st('Open details') }}
            <RiArrowRightSLine class="ml-1 h-4 w-4" />
          </Button>
        </template>
        <dl class="divide-y divide-border/50">
          <div
            v-for="row in card.rows"
            :key="row.label"
            class="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0"
          >
            <dt class="text-sm text-muted-foreground">{{ row.label }}</dt>
            <dd class="text-sm font-medium">{{ modeLabel(row.mode) }}</dd>
          </div>
        </dl>
      </SettingsCard>
    </div>
  </section>
</template>
