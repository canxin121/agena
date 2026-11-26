<script setup lang="ts">
import { sessionApiPath } from '@/lib/acpSession'
import { onMounted, onScopeDispose, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import type { ExternalAgentWorkItem } from '@/types/chat'

const { sessionId } = defineProps<{ sessionId: string }>()
const { t } = useI18n()
const toasts = useToastsStore()
const work = ref<ExternalAgentWorkItem[]>([])
const canceling = ref('')
const error = ref('')
const controller = new AbortController()
const path = `${sessionApiPath(sessionId)}/agent-work`
let timer: number | undefined
let refreshing = false
let revision = 0

async function refresh() {
  if (refreshing || canceling.value) return
  refreshing = true
  const started = revision
  try {
    const items = await apiJson<ExternalAgentWorkItem[]>(path, { signal: controller.signal })
    if (started === revision) work.value = items
  } catch {
    if (!controller.signal.aborted && started === revision) work.value = []
  } finally {
    refreshing = false
  }
}

async function cancel(item: ExternalAgentWorkItem) {
  if (canceling.value) return
  canceling.value = item.cancel_id
  revision += 1
  error.value = ''
  try {
    work.value = await apiJson<ExternalAgentWorkItem[]>(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ cancel_id: item.cancel_id }),
      signal: controller.signal,
    })
    toasts.push('info', t('chat.externalAgentDetails.cancellationRequested'))
  } catch (reason) {
    if (!controller.signal.aborted) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    canceling.value = ''
    if (!controller.signal.aborted) void refresh()
  }
}

onMounted(() => {
  void refresh()
  timer = window.setInterval(refresh, 1500)
})
onScopeDispose(() => {
  window.clearInterval(timer)
  controller.abort()
})
</script>

<template>
  <section v-if="work.length || error" class="grid gap-1.5" :aria-label="t('chat.externalAgentDetails.cancellableWork')">
    <p v-if="work.length" class="font-medium">{{ t('chat.externalAgentDetails.cancellableWork') }}</p>
    <p v-if="error" role="alert" class="break-words text-destructive">{{ error }}</p>
    <ul v-if="work.length" class="grid gap-1.5">
      <li v-for="item in work" :key="item.cancel_id" class="flex items-center justify-between gap-3 rounded border border-border p-2">
        <div class="min-w-0">
          <p class="break-words font-medium">{{ item.title || item.target_id }}</p>
          <p class="text-muted-foreground">{{ t(`chat.externalAgentDetails.workKind.${item.kind}`) }}</p>
        </div>
        <Button type="button" size="sm" variant="outline" :disabled="!!canceling" @click="cancel(item)">
          {{ t('chat.externalAgentDetails.cancelWork') }}
        </Button>
      </li>
    </ul>
  </section>
</template>
