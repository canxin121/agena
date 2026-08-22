<script setup lang="ts">
import { computed } from 'vue'

import { buildWorkspaceRawFileUrl } from '@/lib/workspaceLinks'
import type { AttachmentPresentation } from '@/pages/chat/transcriptPartPresentation'

const props = withDefaults(
  defineProps<{
    attachment: AttachmentPresentation
    workspaceRoot?: string
  }>(),
  { workspaceRoot: '' },
)

const emit = defineEmits<{
  (event: 'open', path: string, url: string): void
}>()

function isBrowserUrl(value: string): boolean {
  return /^(?:https?:|data:|blob:)/i.test(value.trim())
}

const href = computed(() => {
  const url = props.attachment.url.trim()
  const path = props.attachment.path.trim()
  if (isBrowserUrl(url)) return url
  if (path && props.workspaceRoot.trim()) return buildWorkspaceRawFileUrl(props.workspaceRoot.trim(), path)
  return ''
})

const displayMime = computed(() => props.attachment.mime || props.attachment.kind)
const isImage = computed(
  () =>
    props.attachment.mime.startsWith('image/') ||
    /\.(png|jpe?g|gif|webp|avif|svg|bmp|ico|tiff?)$/i.test(props.attachment.label),
)
const isVideo = computed(
  () => props.attachment.mime.startsWith('video/') || /\.(mp4|webm|mov|m4v|ogv)$/i.test(props.attachment.label),
)
const isAudio = computed(
  () =>
    props.attachment.mime.startsWith('audio/') || /\.(mp3|wav|m4a|ogg|flac|aac|opus)$/i.test(props.attachment.label),
)

function formatBytes(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return ''
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB']
  let amount = value / 1024
  let unit = units[0]
  for (let index = 0; index < units.length - 1 && amount >= 1024; index += 1) {
    amount /= 1024
    unit = units[index + 1] || unit
  }
  return `${amount < 10 ? amount.toFixed(1) : Math.round(amount)} ${unit}`
}

const dimensions = computed(() => {
  const width = props.attachment.width
  const height = props.attachment.height
  if (width === null || height === null) return ''
  return `${width}×${height}`
})

const duration = computed(() => {
  const value = props.attachment.durationMs
  if (value === null || !Number.isFinite(value) || value < 0) return ''
  const seconds = value / 1000
  return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)} s`
})

const facts = computed(() =>
  [displayMime.value, formatBytes(props.attachment.sizeBytes), dimensions.value, duration.value]
    .filter(Boolean)
    .join(' · '),
)

function openAttachment() {
  emit('open', props.attachment.path, href.value || props.attachment.url)
}
</script>

<template>
  <section class="min-w-0 border-y border-border/50 py-1.5">
    <div class="flex min-w-0 items-start gap-2">
      <button
        v-if="attachment.path"
        type="button"
        class="min-w-0 flex-1 truncate text-left font-mono text-xs text-foreground hover:text-primary"
        :title="attachment.label"
        @click="openAttachment"
      >
        <span aria-hidden="true">›</span>
        {{ attachment.label }}
      </button>
      <a
        v-else-if="href"
        :href="href"
        target="_blank"
        rel="noopener noreferrer"
        class="min-w-0 flex-1 truncate font-mono text-xs text-foreground hover:text-primary"
        :title="attachment.label"
      >
        <span aria-hidden="true">›</span>
        {{ attachment.label }}
      </a>
      <span v-else class="min-w-0 flex-1 truncate font-mono text-xs" :title="attachment.label">
        <span aria-hidden="true">›</span>
        {{ attachment.label }}
      </span>
      <span v-if="facts" class="shrink-0 text-right text-[10px] text-muted-foreground">{{ facts }}</span>
    </div>

    <img
      v-if="isImage && href"
      :src="href"
      :alt="attachment.label"
      class="mt-2 max-h-96 max-w-full cursor-zoom-in rounded-md object-contain"
      @click="openAttachment"
    />
    <video
      v-else-if="isVideo && href"
      :src="href"
      controls
      preload="metadata"
      class="mt-2 max-h-96 max-w-full rounded-md"
    />
    <audio v-else-if="isAudio && href" :src="href" controls preload="metadata" class="mt-2 w-full" />
    <div v-if="attachment.pageCount !== null" class="mt-1 text-[10px] text-muted-foreground">
      {{ attachment.pageCount }} pages
    </div>
  </section>
</template>
