<script setup lang="ts">
import { computed } from 'vue'
import Markdown from '@/components/Markdown.vue'
import InlineReference from '@/features/inlineUi/InlineReference.vue'
import { splitInlineReferences, inlineMarkdownReferences, mergeInlineReferences } from '@/features/inlineUi/references'

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
  },
)
const references = computed(() =>
  props.allowInlineUi && props.content.includes('inline-ui') ? inlineMarkdownReferences(props.content) : undefined,
)
const documents = computed(() => mergeInlineReferences(null, props.inlineDocuments))
const segments = computed(() =>
  props.mode !== 'plain' && props.allowInlineUi
    ? splitInlineReferences(props.content, props.stream)
    : [{ kind: 'markdown' as const, key: 'text:0', content: props.content }],
)
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
      <Markdown
        v-else
        :content="segment.content"
        :references="references"
        :mode="mode"
        :stream="stream"
        :stream-debounce-ms="streamDebounceMs"
        :source-path="sourcePath"
        :session-id="inlineSessionId"
        :reveal-anchor="revealAnchor"
        :reveal-request-seq="revealRequestSeq"
      />
    </template>
  </div>
</template>
