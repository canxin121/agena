<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowRightSLine, RiGitBranchLine, RiLoader4Line, RiMessageLine } from '@remixicon/vue'

import { getIntlLocale } from '@/i18n/intl'
import type { HubRowKind, SessionResource } from './types'

const props = withDefaults(
  defineProps<{
    session: SessionResource
    kind: HubRowKind
    /** Reference timestamp for relative times; updates keep labels fresh. */
    now?: number
  }>(),
  { now: () => Date.now() },
)

const emit = defineEmits<{ open: [] }>()

const { t } = useI18n()

const title = computed(() => {
  const raw = typeof props.session.title === 'string' ? props.session.title.trim() : ''
  return raw || String(t('hub.untitled'))
})

function parseTs(iso?: string | null): number {
  if (!iso) return 0
  const ms = Date.parse(iso)
  return Number.isFinite(ms) ? ms : 0
}

const timeMs = computed(
  () =>
    parseTs(props.session.last_message_at) || parseTs(props.session.updated_at) || parseTs(props.session.created_at),
)

const timeText = computed(() => formatRelativeTime(timeMs.value, props.now))

function formatRelativeTime(ms: number, now: number): string {
  if (!ms || !Number.isFinite(ms)) return ''
  const diffSec = Math.round((ms - now) / 1000)
  const abs = Math.abs(diffSec)
  let rtf: Intl.RelativeTimeFormat
  try {
    rtf = new Intl.RelativeTimeFormat(getIntlLocale(), { numeric: 'auto' })
  } catch {
    return new Date(ms).toLocaleString()
  }
  if (abs < 60) return rtf.format(diffSec, 'second')
  if (abs < 3600) return rtf.format(Math.round(diffSec / 60), 'minute')
  if (abs < 86400) return rtf.format(Math.round(diffSec / 3600), 'hour')
  if (abs < 86400 * 30) return rtf.format(Math.round(diffSec / 86400), 'day')
  if (abs < 86400 * 365) return rtf.format(Math.round(diffSec / (86400 * 30)), 'month')
  return rtf.format(Math.round(diffSec / (86400 * 365)), 'year')
}

const badge = computed<{ label: string; class: string; spinner: boolean } | null>(() => {
  if (props.kind === 'attention') {
    return {
      label: String(t('hub.attention')),
      class:
        'border-amber-300/40 bg-amber-200/10 text-amber-600 dark:border-amber-400/30 dark:bg-amber-400/10 dark:text-amber-300',
      spinner: false,
    }
  }
  if (props.kind === 'running') {
    return {
      label: String(t('hub.running')),
      class: 'border-sky-300/40 bg-sky-200/10 text-sky-600 dark:border-sky-400/30 dark:bg-sky-400/10 dark:text-sky-300',
      spinner: true,
    }
  }
  return null
})

const messageCount = computed(() => Math.max(0, Number(props.session.message_count) || 0))
const childCount = computed(() => Math.max(0, Number(props.session.child_session_count) || 0))
</script>

<template>
  <button
    type="button"
    class="group flex w-full items-center gap-3 rounded-lg border border-border/60 bg-background px-3 py-2.5 text-left transition-colors hover:border-border hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
    @click="emit('open')"
  >
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <span class="truncate text-sm font-medium text-foreground">{{ title }}</span>
        <span
          v-if="badge"
          class="inline-flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-medium leading-none"
          :class="badge.class"
        >
          <RiLoader4Line v-if="badge.spinner" class="h-3 w-3 animate-spin" />
          {{ badge.label }}
        </span>
      </div>
      <div class="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
        <span v-if="timeText">{{ timeText }}</span>
        <span v-if="messageCount > 0" class="inline-flex items-center gap-1">
          <RiMessageLine class="h-3 w-3" />
          {{ messageCount }}
        </span>
        <span v-if="childCount > 0" class="inline-flex items-center gap-1">
          <RiGitBranchLine class="h-3 w-3" />
          {{ childCount }}
        </span>
      </div>
    </div>
    <RiArrowRightSLine
      class="h-4 w-4 shrink-0 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5 group-hover:text-muted-foreground"
    />
  </button>
</template>
