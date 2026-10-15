<script setup lang="ts">
import Markdown from '@/components/Markdown.vue'
import { computed, defineAsyncComponent } from 'vue'
import { splitReplySegments } from '@/features/interactive/model'
const InteractiveAppCard = defineAsyncComponent(() => import('@/features/interactive/InteractiveAppCard.vue'))
const ReplyChart = defineAsyncComponent(() => import('@/features/interactive/ReplyChart.vue'))

const props = withDefaults(
  defineProps<{
    content: string
    mode?: 'markdown' | 'plain'
    stream?: boolean
    streamDebounceMs?: number
    sourcePath?: string
    revealAnchor?: string
    revealRequestSeq?: number
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
    allowApps: false,
    appOwner: '',
    appDirectory: '',
  },
)
const segments = computed(() =>
  props.mode === 'plain'
    ? [{ kind: 'markdown' as const, key: 'md:0', content: props.content }]
    : splitReplySegments(props.content, props.allowApps),
)
</script>

<template>
  <div class="min-w-0">
    <template v-for="segment in segments" :key="segment.key">
      <InteractiveAppCard
        v-if="segment.kind === 'app'"
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
