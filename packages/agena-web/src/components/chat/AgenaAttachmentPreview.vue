<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiArrowRightSLine,
  RiExternalLinkLine,
  RiFileTextLine,
  RiImageLine,
  RiVideoLine,
  RiVolumeUpLine,
} from '@remixicon/vue'

import { buildWorkspaceRawFileUrl } from '@/lib/workspaceLinks'
import type { AttachmentPresentation } from '@/pages/chat/transcriptPartPresentation'

const props = withDefaults(
  defineProps<{
    attachment: AttachmentPresentation
    workspaceRoot?: string
  }>(),
  { workspaceRoot: '' },
)

const { t } = useI18n()

const emit = defineEmits<{
  (event: 'open', path: string, url: string): void
}>()

function isBrowserUrl(value: string): boolean {
  return /^(?:https?:|data:|blob:)/i.test(value.trim())
}

const href = computed(() => {
  if (props.attachment.unavailable) return ''
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
const unavailable = computed(() => props.attachment.unavailable)
const unavailableNotice = computed(() =>
  t('chat.attachments.unavailable', {
    reason: props.attachment.unavailableReason.trim() || t('chat.attachments.unavailableUnknown'),
  }),
)
const attachmentIcon = computed(() =>
  isImage.value ? RiImageLine : isVideo.value ? RiVideoLine : isAudio.value ? RiVolumeUpLine : RiFileTextLine,
)
const fullTitle = computed(() =>
  [props.attachment.label, unavailable.value ? unavailableNotice.value : facts.value].filter(Boolean).join(' — '),
)

function openAttachment() {
  emit('open', props.attachment.path, href.value || props.attachment.url)
}
</script>

<template>
  <section class="min-w-0 space-y-2 py-1">
    <div class="interaction-width">
      <button
        tabindex="0"
        v-if="attachment.path && !unavailable"
        type="button"
        class="part-link"
        :title="fullTitle"
        :aria-label="`${t('common.open')}: ${attachment.label}`"
        :aria-description="facts || undefined"
        @click="openAttachment"
      >
        <span class="interaction-choice-marker" aria-hidden="true"><component :is="attachmentIcon" /></span>
        <span class="interaction-choice-copy">
          <span class="interaction-choice-label font-mono">{{ attachment.label }}</span>
          <span v-if="facts" class="interaction-choice-description">{{ facts }}</span>
        </span>
        <RiArrowRightSLine class="interaction-choice-arrow" aria-hidden="true" />
      </button>
      <a
        tabindex="0"
        v-else-if="href"
        :href="href"
        target="_blank"
        rel="noopener noreferrer"
        class="part-link"
        :title="fullTitle"
        :aria-label="`${t('common.open')}: ${attachment.label}`"
        :aria-description="facts || undefined"
      >
        <span class="interaction-choice-marker" aria-hidden="true"><component :is="attachmentIcon" /></span>
        <span class="interaction-choice-copy">
          <span class="interaction-choice-label font-mono">{{ attachment.label }}</span>
          <span v-if="facts" class="interaction-choice-description">{{ facts }}</span>
        </span>
        <RiExternalLinkLine class="interaction-choice-arrow" aria-hidden="true" />
      </a>
      <div v-else class="flex min-w-0 items-center gap-2 py-1.5" :title="fullTitle">
        <component :is="attachmentIcon" class="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div class="min-w-0">
          <p class="truncate font-mono text-xs font-medium">{{ attachment.label }}</p>
          <p v-if="unavailable" class="text-[11px] text-muted-foreground">{{ unavailableNotice }}</p>
          <p v-else-if="facts" class="text-[11px] text-muted-foreground">{{ facts }}</p>
        </div>
      </div>
    </div>

    <button
      tabindex="0"
      v-if="isImage && href"
      type="button"
      class="relative block max-w-full overflow-hidden rounded-md border border-border bg-card outline-none focus-visible:ring-2 focus-visible:ring-ring"
      :aria-label="`${t('common.open')}: ${attachment.label}`"
      @click="openAttachment"
    >
      <img :src="href" alt="" aria-hidden="true" class="max-h-96 max-w-full cursor-zoom-in object-contain" />
      <span
        class="pointer-events-none absolute bottom-2 right-2 flex items-center gap-1 rounded border border-border bg-background/90 px-1.5 py-1 text-[11px]"
        aria-hidden="true"
      >
        <RiImageLine class="h-3 w-3" />{{ t('common.open') }}
      </span>
    </button>
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
