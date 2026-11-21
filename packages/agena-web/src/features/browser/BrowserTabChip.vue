<script setup lang="ts">
import { computed, ref } from 'vue'
import { useIntersectionObserver } from '@vueuse/core'
import { RiGlobalLine, RiHand } from '@remixicon/vue'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { limitBackgroundReads } from '@/lib/backgroundReads'
import { browserCommand, type BrowserState } from './api'
import { useBrowserCopy } from './copy'

const props = defineProps<{
  sessionId: string
  tab: BrowserState['tabs'][number]
  human: boolean
  requested: boolean
  viewerOpen: boolean
}>()
const emit = defineEmits<{ open: [pageId: string] }>()
const t = useBrowserCopy()
const trigger = ref<HTMLButtonElement>()
const onscreen = ref(false)
useIntersectionObserver(trigger, ([entry]) => {
  onscreen.value = Boolean(entry?.isIntersecting)
})
const title = computed(() => props.tab.title.replace(/^Browser · /, '') || t('browser'))
const caption = computed(() => (props.requested ? t('requested') : props.human ? t('manualControl') : t('browser')))
const { data: image, error } = useVisibleResource({
  key: computed(() => (onscreen.value && !props.viewerOpen && !props.human ? props.tab.page_id : '')),
  interval: () => 5000,
  async load(page, signal) {
    const frame = await limitBackgroundReads(
      () => browserCommand(props.sessionId, { kind: 'frame', page_id: page, thumbnail: true }, signal),
      signal,
    )
    return frame.kind === 'frame' ? frame.jpeg : ''
  },
})
</script>

<template>
  <button
    ref="trigger"
    type="button"
    class="composer-resource"
    :title="title"
    :aria-label="`${t('open')} · ${title}`"
    aria-haspopup="dialog"
    :data-browser-tab="tab.page_id"
    @click="emit('open', tab.page_id)"
  >
    <span class="grid h-9 w-16 shrink-0 place-items-center overflow-hidden rounded border border-border bg-muted/50">
      <img
        v-if="image && !error"
        :src="'data:image/jpeg;base64,' + image"
        alt=""
        class="h-full w-full object-contain"
        :draggable="false"
      />
      <component
        v-else
        :is="human || requested ? RiHand : RiGlobalLine"
        class="h-4 w-4 text-muted-foreground"
        aria-hidden="true"
      />
    </span>
    <span class="min-w-0 text-start">
      <span class="resource-name">{{ title }}</span>
      <span class="resource-caption" :class="{ 'text-primary': requested }">{{ caption }}</span>
    </span>
  </button>
</template>
