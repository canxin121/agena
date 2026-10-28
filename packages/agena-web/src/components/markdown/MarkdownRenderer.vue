<script setup lang="ts">
import InlineReference from '@/features/inlineUi/InlineReference.vue'
import {
  splitInlineReferences,
  inlineMarkdownReferences,
  mergeInlineReferences,
  type InlineTextSegment,
} from '@/features/inlineUi/references'
import Markdown from '@/components/Markdown.vue'
import { computed, defineAsyncComponent } from 'vue'
import { splitReplySegments, type ReplySegment } from '@/features/interactive/model'
const InteractiveAppCard = defineAsyncComponent(() => import('@/features/interactive/InteractiveAppCard.vue'))
const ReplyChart = defineAsyncComponent(() => import('@/features/inlineUi/InlineChart.vue'))

const props = withDefaults(
  defineProps<{
    content: string
    mode?: 'markdown' | 'plain'
    stream?: boolean
    streamDebounceMs?: number
    sourcePath?: string
    revealAnchor?: string
    revealRequestSeq?: number
    allowInlineUi?: boolean
    inlineDocuments?: unknown
    inlineSessionId?: string
    allowApps?: boolean
    appOwner?: string
    appDirectory?: string
  }>(),
  {
    mode: 'markdown',
    stream: false,
    streamDebounceMs: 60,
    sourcePath: '',
    revealAnchor: '',
    revealRequestSeq: 0,
    allowInlineUi: false,
    inlineDocuments: null,
    inlineSessionId: '',
    allowApps: false,
    appOwner: '',
    appDirectory: '',
  },
)
const references = computed(() =>
  props.allowInlineUi && props.content.includes('[[inline-ui:') ? inlineMarkdownReferences(props.content) : undefined,
)
const documents = computed(() => mergeInlineReferences(null, props.inlineDocuments))
const segments = computed<Array<ReplySegment | Exclude<InlineTextSegment, { kind: 'markdown' }>>>(() => {
  if (props.mode === 'plain') return [{ kind: 'markdown' as const, key: 'md:0', content: props.content }]
  if (!props.allowInlineUi) return splitReplySegments(props.content, props.allowApps)
  const result: Array<ReplySegment | Exclude<InlineTextSegment, { kind: 'markdown' }>> = []
  for (const segment of splitInlineReferences(props.content, props.stream)) {
    if (segment.kind !== 'markdown') result.push(segment)
    else
      result.push(
        ...splitReplySegments(segment.content, props.allowApps).map((child) => ({
          ...child,
          key: `${segment.key}:${child.key}`,
        })),
      )
  }
  return result
})
</script>

<template>
  <div class="min-w-0">
    <template v-for="segment in segments" :key="segment.key">
      <InlineReference
        v-if="segment.kind === 'reference'"
        :reference="segment.reference"
        :session-id="inlineSessionId"
        :documents="documents"
      />
      <span v-else-if="segment.kind === 'pending-reference'" aria-hidden="true" data-inline-reference-pending />
      <InteractiveAppCard
        v-else-if="segment.kind === 'app'"
        :manifest="segment.manifest"
        :owner="appOwner"
        :directory="appDirectory"
      />
      <ReplyChart v-else-if="segment.kind === 'chart'" :spec="segment.spec" :source="segment.source" />
      <template v-else>
        <p v-if="'error' in segment && segment.error" class="my-2 text-xs text-muted-foreground" role="status">
          {{ segment.error }}
        </p>
        <Markdown
          :content="segment.content"
          :references="references"
          :mode="mode"
          :stream="stream"
          :stream-debounce-ms="streamDebounceMs"
          :source-path="sourcePath"
          :reveal-anchor="revealAnchor"
          :reveal-request-seq="revealRequestSeq"
        />
      </template>
    </template>
  </div>
</template>
