<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCheckLine, RiFileCopyLine, RiRefreshLine } from '@remixicon/vue'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import AgenaOperationPart from '@/components/chat/AgenaOperationPart.vue'
import { getToolPartDetail } from '@/stores/chat/api'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import InlineReply from './InlineReply.vue'
import { inlineDocumentFromPresentation, inlineDocumentMarkdown } from './model'

const props = defineProps<{ part: TranscriptDisplayPart; sessionId?: string | null }>()
const { t } = useI18n()
const pane = useWorkspacePaneContext()
const fetched = shallowRef<unknown>(null)
const error = ref('')
const loading = ref(false)
const copied = ref(false)
const inspect = ref(false)
const status = computed(() => partStatusPresentation(props.part.status))
const documentValue = computed(
  () =>
    inlineDocumentFromPresentation(props.part.source.agenaPresentation) ||
    inlineDocumentFromPresentation(fetched.value),
)
let controller: AbortController | undefined
let generation = 0
let disposed = false
let copyTimer: ReturnType<typeof setTimeout> | undefined

async function load(force = false) {
  if (
    disposed ||
    documentValue.value ||
    loading.value ||
    !status.value.terminal ||
    !props.sessionId ||
    window.document.hidden ||
    (pane && !pane.isVisible.value)
  )
    return
  controller?.abort()
  const current = ++generation
  controller = new AbortController()
  loading.value = true
  error.value = ''
  try {
    const result = await getToolPartDetail(
      props.sessionId,
      String(props.part.id),
      'presentation',
      controller.signal,
      force,
    )
    if (disposed || current !== generation) return
    fetched.value = result.value
    if (!inlineDocumentFromPresentation(result.value)) error.value = t('inlineUi.unavailable')
  } catch (err) {
    if (!disposed && current === generation && !controller.signal.aborted)
      error.value = err instanceof Error ? err.message : t('inlineUi.unavailable')
  } finally {
    if (current === generation) loading.value = false
  }
}
watch(
  () => [props.sessionId, props.part.id, props.part.source.revision, props.part.status],
  () => {
    controller?.abort()
    generation++
    loading.value = false
    fetched.value = null
    error.value = ''
    copied.value = false
    void load()
  },
  { immediate: true },
)
watch(
  () => pane?.isVisible.value,
  () => {
    void load()
  },
)
function visibility() {
  if (!window.document.hidden) void load()
}
onMounted(() => {
  window.document.addEventListener('visibilitychange', visibility)
  void load()
})
onBeforeUnmount(() => {
  disposed = true
  generation++
  controller?.abort()
  clearTimeout(copyTimer)
  window.document.removeEventListener('visibilitychange', visibility)
})
async function copy() {
  if (!documentValue.value) return
  try {
    if (!(await copyTextToClipboard(inlineDocumentMarkdown(documentValue.value))))
      throw new Error('clipboard unavailable')
    copied.value = true
    clearTimeout(copyTimer)
    copyTimer = setTimeout(() => {
      copied.value = false
    }, 1800)
  } catch {
    error.value = t('inlineUi.copyFailed')
  }
}
</script>

<template>
  <div class="inline-reply-part my-3 min-w-0" data-inline-reply-part>
    <InlineReply v-if="documentValue" :document="documentValue" />
    <p v-else-if="loading || !status.terminal" class="py-2 text-sm text-muted-foreground" role="status">
      {{ t('inlineUi.loading') }}
    </p>
    <p v-if="error" class="py-2 text-xs text-muted-foreground" role="status">{{ error }}</p>
    <div v-if="status.terminal" class="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
      <button v-if="documentValue" type="button" class="inline-action" @click="copy">
        <component :is="copied ? RiCheckLine : RiFileCopyLine" class="h-3.5 w-3.5" aria-hidden="true" />{{
          t(copied ? 'inlineUi.copied' : 'inlineUi.copy')
        }}
      </button>
      <button v-else type="button" class="inline-action" :disabled="loading" @click="load(true)">
        <RiRefreshLine class="h-3.5 w-3.5" aria-hidden="true" />{{ t('inlineUi.retry') }}
      </button>
      <button type="button" class="inline-action" :aria-expanded="inspect" @click="inspect = !inspect">
        {{ t('inlineUi.inspect') }}
      </button>
    </div>
    <AgenaOperationPart
      v-if="inspect"
      :part="part"
      :session-id="sessionId"
      :expanded="true"
      :collapse-signal="0"
      @toggle="inspect = false"
    />
  </div>
</template>

<style scoped>
.inline-action {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  min-height: 1.75rem;
  padding: 0.1rem 0.25rem;
  border-radius: 0.25rem;
  opacity: 0.8;
}
.inline-action:hover {
  color: var(--color-foreground);
  opacity: 1;
}
.inline-action:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: 2px;
}
</style>
