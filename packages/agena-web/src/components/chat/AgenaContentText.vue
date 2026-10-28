<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AgenaDiffBlock from './AgenaDiffBlock.vue'
import PartLoadingIndicator from './PartLoadingIndicator.vue'
import { type ContentFormat, type ContentRef } from '@/lib/content'
import { observeContent } from '@/lib/contentSubscriptions'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useChatStore } from '@/stores/chat'

const props = defineProps<{
  resource: ContentRef
  sessionId: string
  plain?: boolean
  sourcePath?: string
  format?: ContentFormat
  allowApps?: boolean
}>()
const { t } = useI18n()
const pane = useWorkspacePaneContext()
const chat = useChatStore()
const body = shallowRef('')
const active = ref(true)
const gap = ref(false)
const error = ref('')
const loading = ref(true)
const root = ref<HTMLElement>()
const inlineDocuments = shallowRef<unknown>(null)
let latestBody = ''
function selected() {
  const selection = document.getSelection()
  return !!(selection && !selection.isCollapsed && selection.anchorNode && root.value?.contains(selection.anchorNode))
}
function updateSelection() {
  if (pane && !pane.isVisible.value) return
  if (!selected()) body.value = latestBody
}
const jsonBody = computed(() => {
  if (props.format !== 'json') return body.value
  try {
    return JSON.stringify(JSON.parse(body.value), null, 2)
  } catch {
    return body.value
  }
})
let stop: (() => void) | undefined
let mounted = false
function connect() {
  stop?.()
  stop = undefined
  if (pane && !pane.isVisible.value) return
  stop = observeContent(props.sessionId, props.resource, (frame) => {
    inlineDocuments.value = frame.buffer.inlineUi
    latestBody = frame.text
    if (!selected()) body.value = latestBody
    active.value =
      !frame.buffer.resource ||
      frame.buffer.resource.state === 'active' ||
      (frame.buffer.cursor?.sequence ?? 0) < frame.buffer.resource.cursor.sequence
    gap.value = frame.buffer.gap
    error.value = frame.error
    loading.value = !frame.buffer.resource && !frame.text && !frame.error
  })
}
if (pane)
  watch(pane.isVisible, () => {
    if (mounted) connect()
  })
onMounted(() => {
  mounted = true
  document.addEventListener('selectionchange', updateSelection)
  connect()
})
watch(
  () => `${props.sessionId}/${props.resource.resource_id}`,
  () => {
    if (mounted) connect()
  },
)
onBeforeUnmount(() => {
  stop?.()
  document.removeEventListener('selectionchange', updateSelection)
})
</script>

<template>
  <div ref="root" :aria-busy="loading" data-content-text>
    <PartLoadingIndicator v-if="loading" class="min-h-8 py-1" />
    <AgenaDiffBlock v-else-if="format === 'diff'" :diff="body" />
    <CodeBlock
      v-else-if="format === 'json' || format === 'code'"
      :code="jsonBody"
      :lang="format === 'json' ? 'json' : 'text'"
      compact
    />
    <MarkdownRenderer
      v-else-if="format === 'markdown' || (!format && !plain)"
      :content="body"
      :stream="active"
      :source-path="sourcePath || ''"
      :allow-apps="allowApps"
      :allow-inline-ui="allowApps === true"
      :inline-documents="inlineDocuments"
      :inline-session-id="sessionId"
      :app-owner="sessionId"
      :app-directory="chat.getSessionDirectory(sessionId) || ''"
    />
    <pre
      v-else
      class="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-snug text-foreground/90"
      >{{ body }}</pre
    >
    <div v-if="gap" class="mt-1 text-xs text-amber-600">{{ t('content.gap') }}</div>
    <div v-if="error" class="mt-1 text-xs text-amber-600" role="status">{{ error }}</div>
  </div>
</template>
