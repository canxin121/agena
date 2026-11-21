<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import { apiJson, ApiError } from '@/lib/api'
import { captureResourceObservation } from '@/lib/resourceSync'
import { readServerAuthTokenVersion, subscribeServerAuthTokenVersion } from '@/lib/serverAuthToken'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'

import { processCollection, type ProcessKind, type SessionProcess } from '@/pages/chat/sessionProcesses'

const props = defineProps<{ sessionId: string; id: string; kind: ProcessKind }>()
const emit = defineEmits<{ changed: [process: SessionProcess] }>()
const { t } = useI18n()
const pane = useWorkspacePaneContext()
const state = ref<SessionProcess>()
const stopping = ref(false)
const error = ref('')
const readError = ref('')
const authVersion = ref(readServerAuthTokenVersion())
onScopeDispose(subscribeServerAuthTokenVersion((version) => (authVersion.value = version)))
const visible = computed(() => pane?.isVisible.value ?? true)
const endpoint = computed(() => `${processCollection(props.sessionId, props.kind)}/${encodeURIComponent(props.id)}`)
let controller: AbortController | undefined
let revision = 0
let observedScope = ''

watch(
  [endpoint, visible, authVersion],
  ([url, isVisible, auth], previous, cleanup) => {
    if (url !== previous?.[0] || auth !== previous?.[2]) {
      state.value = undefined
      error.value = ''
      readError.value = ''
    }
    stopping.value = false
    if (!isVisible) return

    const request = new AbortController()
    controller = request
    let timer: ReturnType<typeof setTimeout> | undefined
    cleanup(() => {
      request.abort()
      clearTimeout(timer)
      if (controller === request) controller = undefined
    })

    async function refresh() {
      const observed = revision
      const scope = captureResourceObservation(url).scope
      try {
        const updated = await apiJson<SessionProcess>(url, {
          cache: 'no-store',
          signal: AbortSignal.any([request.signal, AbortSignal.timeout(5000)]),
        })
        if (request.signal.aborted) return
        if (observed === revision && !stopping.value && scope === captureResourceObservation(url).scope) {
          observedScope = scope
          readError.value = ''
          state.value = updated
        }
      } catch (cause) {
        if (request.signal.aborted) return
        // Finished output belongs to the part and can outlive the process registry.
        if (cause instanceof ApiError && cause.status === 404) {
          if (observed === revision && !stopping.value) {
            state.value = undefined
            readError.value = ''
          }
          return
        }
        if (observed === revision && !stopping.value && scope === captureResourceObservation(url).scope) {
          readError.value = cause instanceof Error ? cause.message : String(cause)
        }
      }
      if (!state.value || state.value.status === 'running') timer = setTimeout(refresh, readError.value ? 5000 : 1000)
    }

    void refresh()
  },
  { immediate: true },
)

async function stop() {
  const request = controller
  if (!request || stopping.value || state.value?.status !== 'running') return
  const url = endpoint.value
  const scope = captureResourceObservation(url).scope
  if (scope !== observedScope) {
    state.value = undefined
    return
  }
  ++revision
  stopping.value = true
  error.value = ''
  try {
    const updated = await apiJson<SessionProcess>(url, {
      method: 'DELETE',
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(10_000)]),
    })
    if (!request.signal.aborted && scope === captureResourceObservation(url).scope) {
      state.value = updated
      emit('changed', updated)
    }
  } catch (cause) {
    if (!request.signal.aborted) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (!request.signal.aborted) {
      ++revision
      stopping.value = false
    }
  }
}
</script>

<template>
  <div v-if="state || error || readError" class="flex flex-wrap items-center gap-2 py-0.5 text-xs">
    <template v-if="state">
      <span class="font-medium text-foreground">
        {{ t(kind === 'shell' ? 'chat.operations.process' : 'chat.operations.subscription') }}
      </span>
      <span class="text-muted-foreground">
        {{
          kind === 'shell' && state.exit_code != null
            ? t('chat.activityLog.exitCode', { code: state.exit_code })
            : t(`chat.operations.status.${state.status}`)
        }}
      </span>
      <span v-if="kind === 'shell' && state.ready && state.status === 'running'" class="text-muted-foreground">
        {{ t('chat.operations.ready') }}
      </span>
      <Button v-if="state.status === 'running'" size="sm" variant="outline" :disabled="stopping" @click="stop">
        {{ t(kind === 'shell' ? 'chat.operations.stopProcess' : 'chat.operations.disconnect') }}
      </Button>
    </template>
    <span v-if="error || readError" class="text-destructive" role="alert">{{ error || readError }}</span>
  </div>
</template>
