<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiLoader4Line, RiRefreshLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Dialog from '@/components/ui/Dialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { apiJson } from '@/lib/api'
import { buildPlanToolInvocationRequest, type PlanTool, type PlanToolInput } from '@/pages/chat/planViewerRequest'
import type { JsonValue } from '@/types/json'

const props = defineProps<{
  open: boolean
  sessionId: string | null
}>()

const emit = defineEmits<{
  (event: 'update:open', open: boolean): void
}>()

const { t } = useI18n()

type JsonRecord = Record<string, JsonValue>

const loading = ref(false)
const toggling = ref(false)
const markdown = ref('')
const error = ref('')
const autorun = ref<boolean | null>(null)
let requestSerial = 0

function record(value: JsonValue | null | undefined): JsonRecord {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonRecord) : {}
}

async function invokePlanTool(tool: PlanTool, input: PlanToolInput): Promise<JsonRecord> {
  const body = buildPlanToolInvocationRequest(props.sessionId, tool, input)
  if (!body) throw new Error(String(t('chat.planViewer.requiresSession')))
  const response = await apiJson<JsonValue>('/api/v1/plugins/ui/invoke-tool', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  return record(response)
}

function readAutorun(response: JsonRecord): boolean | null {
  const plan = record(record(response.payload).plan)
  return typeof plan.autorun === 'boolean' ? plan.autorun : null
}

async function refresh() {
  const serial = ++requestSerial
  loading.value = true
  error.value = ''
  try {
    const response = await invokePlanTool('get', { view: 'full' })
    if (serial !== requestSerial) return
    markdown.value = typeof response.output_text === 'string' ? response.output_text.trim() : ''
    autorun.value = readAutorun(response)
  } catch (reason) {
    if (serial !== requestSerial) return
    markdown.value = ''
    autorun.value = null
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (serial === requestSerial) loading.value = false
  }
}

async function toggleAutorun() {
  if (autorun.value === null || toggling.value) return
  toggling.value = true
  error.value = ''
  try {
    await invokePlanTool('phase', { autorun: !autorun.value })
    await refresh()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    toggling.value = false
  }
}

function handleKeydown(event: KeyboardEvent) {
  if (event.metaKey || event.altKey || event.ctrlKey) return
  if (event.key === 'q') {
    event.preventDefault()
    emit('update:open', false)
  } else if (event.key === 'r' || event.key === 'R') {
    event.preventDefault()
    void refresh()
  } else if (event.key === 'a' || event.key === 'A') {
    event.preventDefault()
    void toggleAutorun()
  }
}

const description = computed(() =>
  props.sessionId
    ? String(t('chat.planViewer.session', { id: props.sessionId }))
    : String(t('chat.planViewer.requiresSession')),
)

watch(
  () => [props.open, props.sessionId] as const,
  ([open]) => {
    requestSerial += 1
    if (open) void refresh()
  },
  { immediate: true },
)
</script>

<template>
  <Dialog
    :open="open"
    :title="t('chat.planViewer.title')"
    :description="description"
    max-width="max-w-4xl"
    mobile-fullscreen
    @update:open="$emit('update:open', $event)"
  >
    <div class="flex min-h-0 flex-col" data-transcript-chrome="true" tabindex="-1" @keydown="handleKeydown">
      <div class="flex shrink-0 items-center justify-between gap-3 border-y border-border/60 py-2">
        <label v-if="autorun !== null" class="inline-flex items-center gap-2 text-xs">
          <input type="checkbox" :checked="autorun" :disabled="toggling" class="h-4 w-4" @change="toggleAutorun" />
          <span>{{ t('chat.planViewer.autorun') }}</span>
          <RiLoader4Line v-if="toggling" class="h-3.5 w-3.5 animate-spin text-muted-foreground" />
        </label>
        <span v-else class="text-xs text-muted-foreground">{{ t('chat.planViewer.noAutorun') }}</span>

        <IconButton
          variant="ghost"
          class="h-8 w-8"
          :tooltip="t('chat.planViewer.refresh')"
          :aria-label="t('chat.planViewer.refresh')"
          :disabled="loading"
          @click="refresh"
        >
          <RiLoader4Line v-if="loading" class="h-4 w-4 animate-spin" />
          <RiRefreshLine v-else class="h-4 w-4" />
        </IconButton>
      </div>

      <div class="min-h-[12rem] overflow-auto py-3 sm:max-h-[70vh]">
        <div v-if="loading && !markdown" class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
          <RiLoader4Line class="h-4 w-4 animate-spin" />
          {{ t('chat.planViewer.loading') }}
        </div>
        <div v-else-if="error" class="border-l-2 border-rose-500/60 pl-3 text-sm text-rose-700 dark:text-rose-300">
          {{ error }}
        </div>
        <MarkdownRenderer v-else-if="markdown" :content="markdown" mode="markdown" :stream="false" />
        <div v-else class="py-8 text-sm text-muted-foreground">{{ t('chat.planViewer.empty') }}</div>
      </div>
    </div>
  </Dialog>
</template>
