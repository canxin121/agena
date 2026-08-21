<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiBrainAi3Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ActivityDisclosureButton from '@/components/ui/ActivityDisclosureButton.vue'
import { stripMarkdownToText } from '@/lib/markdown'
import type { JsonValue } from '@/types/json'

type ReasoningPart = {
  text?: string
  content?: string
  [k: string]: JsonValue
}

const props = defineProps<{
  part: ReasoningPart
  initiallyExpanded?: boolean
  // Bump this value to force-close details.
  collapseSignal?: number
}>()

const { t } = useI18n()

const isOpen = ref(Boolean(props.initiallyExpanded))

function toggleOpen() {
  const next = !isOpen.value
  isOpen.value = next
}

function reasoningSummary(text: string): string {
  const trimmed = (text || '').trim()
  if (!trimmed) return ''
  const newlineIndex = trimmed.indexOf('\n')
  const periodIndex = trimmed.indexOf('.')
  const cutoff = Math.min(
    newlineIndex >= 0 ? newlineIndex : Number.POSITIVE_INFINITY,
    periodIndex >= 0 ? periodIndex : Number.POSITIVE_INFINITY,
  )
  if (!Number.isFinite(cutoff)) return trimmed
  return trimmed.substring(0, cutoff).trim()
}

const rawText = computed(() => {
  const p = props.part || {}
  return (typeof p.text === 'string' ? p.text : typeof p.content === 'string' ? p.content : '') as string
})

const textContent = computed(() => stripMarkdownToText(rawText.value))

const label = computed(() => t('chat.messages.activity.reasoningInvocation.types.thinking'))
const Icon = RiBrainAi3Line
const summary = computed(() => reasoningSummary(textContent.value))

const hasText = computed(() => textContent.value.trim().length > 0)

watch(
  () => props.collapseSignal,
  () => {
    isOpen.value = false
  },
)

watch(
  () => props.initiallyExpanded,
  (next) => {
    // If the user changes the default in Settings, only apply it when the panel
    // is currently closed so we don't fight manual toggles.
    if (isOpen.value) return
    isOpen.value = Boolean(next)
  },
)
</script>

<template>
  <div v-if="hasText" class="relative">
    <ActivityDisclosureButton :open="isOpen" :icon="Icon" :label="label" :summary="summary" @toggle="toggleOpen" />

    <Transition name="toolreveal">
      <div v-show="isOpen" class="pl-6 pt-0.5 pb-1">
        <div class="oc-activity-detail">
          <div
            class="max-h-80 overflow-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground/70 px-3 py-2"
          >
            {{ textContent }}
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.toolreveal-enter-active,
.toolreveal-leave-active {
  transition:
    opacity 140ms ease,
    transform 160ms ease;
}

.toolreveal-enter-from,
.toolreveal-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
