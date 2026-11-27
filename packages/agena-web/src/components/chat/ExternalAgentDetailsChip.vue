<script setup lang="ts">
import {
  RiArrowRightUpLine,
  RiCheckLine,
  RiErrorWarningLine,
  RiFileCopyLine,
  RiPulseLine,
  RiSparklingLine,
} from '@remixicon/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import SessionChip from './SessionChip.vue'
import ExternalAgentWorkList from './ExternalAgentWorkList.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useToastsStore } from '@/stores/toasts'
import type { ExternalAgentDetails, ExternalAgentMessage, ExternalAgentSuggestion } from '@/types/chat'

const props = defineProps<{ sessionId: string; label: string; details?: ExternalAgentDetails; suggestionEnabled: boolean }>()
const emit = defineEmits<{ suggestion: [suggestion: ExternalAgentSuggestion] }>()
const { locale, t } = useI18n()
const toasts = useToastsStore()
const open = ref(false)
const liveWorkCount = ref(0)
const copiedTerminalKey = ref('')
let copyTimer: number | undefined

const details = computed<ExternalAgentDetails>(() => props.details ?? {})
const children = computed(() => Object.entries(props.details?.subagents ?? {}))
const amount = computed(() => {
  const cost = props.details?.cost
  return cost ? `${new Intl.NumberFormat(locale.value, { maximumSignificantDigits: 10 }).format(cost.amount)} ${cost.currency}` : ''
})

const runningCount = computed(() => {
  const runningSubagents = children.value.filter(([, child]) => child.state === 'running' || child.state === 'in_progress').length
  const runningCompactions = (details.value.compactions ?? []).filter((item) => item.status === 'running' || item.status === 'in_progress').length
  const runningTerminals = (details.value.terminals ?? []).filter((term) => !term.exited).length
  return runningSubagents + runningCompactions + runningTerminals + liveWorkCount.value
})
const hasErrorNotice = computed(() => (details.value.notices ?? []).some((notice) => notice.severity === 'error'))
const badgeCount = computed(
  () =>
    (details.value.suggestions?.length ?? 0) +
    (details.value.notices?.length ?? 0) +
    (details.value.compactions?.length ?? 0) +
    (details.value.terminals?.length ?? 0) +
    children.value.length +
    liveWorkCount.value,
)
const hasAnyContent = computed(
  () =>
    Boolean(
      amount.value ||
        badgeCount.value > 0 ||
        (details.value.messages?.length ?? 0) > 0,
    ),
)

watch(() => props.details?.notices?.at(-1), (notice, previous) => {
  if (!notice || (previous?.title === notice.title && previous.description === notice.description)) return
  toasts.push(notice.severity === 'error' ? 'error' : 'info', `${props.label} · ${notice.title}`)
})

function agentName(id: string) {
  return props.details?.subagents?.[id]?.title || id
}

function messageLabel(message: ExternalAgentMessage) {
  if (message.sender_id) return agentName(message.sender_id)
  if (message.recipient_id) return t('chat.externalAgentDetails.recipient', { name: agentName(message.recipient_id) }, 'To {name}')
  return t(`chat.externalAgentDetails.roles.${message.role}`, message.role)
}

function useSuggestion(suggestion: ExternalAgentSuggestion) {
  if (!props.suggestionEnabled) return
  open.value = false
  emit('suggestion', suggestion)
}

function statusPillClass(state?: string) {
  if (state === 'running' || state === 'in_progress') return 'border-emerald-500/30 bg-emerald-500/15 text-emerald-700 dark:text-emerald-300'
  if (state === 'failed' || state === 'error') return 'border-destructive/30 bg-destructive/10 text-destructive'
  if (state === 'completed' || state === 'idle' || state === 'end_turn') return 'border-border/70 bg-muted/60 text-muted-foreground'
  return 'border-border/70 bg-muted/40 text-muted-foreground'
}

function noticeBorderClass(severity: string) {
  if (severity === 'error') return 'border-destructive/40 bg-destructive/5'
  if (severity === 'warning') return 'border-amber-500/40 bg-amber-500/5'
  return 'border-border/70 bg-muted/20'
}

async function copyTerminalOutput(key: string, output: string) {
  const ok = await copyTextToClipboard(output)
  if (!ok) return
  copiedTerminalKey.value = key
  toasts.push('success', String(t('common.copied')))
  window.clearTimeout(copyTimer)
  copyTimer = window.setTimeout(() => {
    if (copiedTerminalKey.value === key) copiedTerminalKey.value = ''
  }, 1800)
}
</script>

<template>
  <SessionChip
    v-model:open="open"
    :title="`${label} · ${t('chat.externalAgentDetails.activity', 'Activity')}`"
    :label="amount || t('chat.externalAgentDetails.activity', 'Activity')"
    panel-class="w-[min(92vw,32rem)]"
  >
    <template #trigger>
      <RiPulseLine class="h-3 w-3 shrink-0" :class="runningCount > 0 ? 'text-emerald-500' : ''" />
      <span class="max-w-36 truncate font-mono text-[11px] font-medium">
        {{ amount || t('chat.externalAgentDetails.activity', 'Activity') }}
      </span>
      <span
        v-if="runningCount > 0"
        class="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
        aria-hidden="true"
      />
      <span
        v-else-if="hasErrorNotice"
        class="h-1.5 w-1.5 shrink-0 rounded-full bg-destructive"
        aria-hidden="true"
      />
      <span
        v-else-if="(details.suggestions?.length ?? 0) > 0"
        class="h-1.5 w-1.5 shrink-0 rounded-full bg-primary"
        aria-hidden="true"
      />
      <span
        v-if="badgeCount > 0"
        class="rounded-full bg-muted/80 px-1.5 py-0.2 font-mono text-[10px] tabular-nums text-foreground"
      >
        {{ badgeCount }}
      </span>
    </template>

    <div class="grid gap-3 text-xs">
      <ExternalAgentWorkList v-if="open" :session-id="sessionId" @count="liveWorkCount = $event" />

      <div v-if="details.suggestions?.length" class="grid gap-1.5">
        <p class="flex items-center gap-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          <RiSparklingLine class="h-3.5 w-3.5 text-primary" />
          {{ t('chat.externalAgentDetails.suggestions') }}
        </p>
        <button
          v-for="(suggestion, i) in details.suggestions"
          :key="i"
          type="button"
          class="group flex items-start justify-between gap-2 rounded-lg border border-border/80 bg-muted/20 p-2.5 text-start transition-colors hover:border-primary/40 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
          :disabled="!suggestionEnabled"
          @click="useSuggestion(suggestion)"
        >
          <span class="min-w-0 flex-1 space-y-0.5">
            <template v-for="(block, j) in suggestion.prompt" :key="j">
              <span v-if="block.type === 'text'" class="block whitespace-pre-wrap break-words text-foreground">{{ block.text }}</span>
              <span v-else class="block break-all font-mono text-[11px] text-muted-foreground">{{ block.name }} · {{ block.uri }}</span>
            </template>
          </span>
          <RiArrowRightUpLine class="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground group-hover:text-primary" />
        </button>
      </div>

      <div v-if="amount" class="flex items-center justify-between gap-3 rounded-md border border-border/60 bg-muted/20 px-2.5 py-2">
        <span class="text-muted-foreground">{{ t('chat.externalAgentDetails.cost', 'Agent-reported total') }}</span>
        <span class="font-mono font-medium text-foreground">{{ amount }}</span>
      </div>

      <ul v-if="details.notices?.length" class="grid gap-1.5" aria-live="polite">
        <li
          v-for="(notice, index) in details.notices"
          :key="index"
          class="rounded-md border px-2.5 py-2"
          :class="noticeBorderClass(notice.severity)"
        >
          <p class="flex items-center gap-1.5 break-words font-medium" :class="{ 'text-destructive': notice.severity === 'error' }">
            <RiErrorWarningLine v-if="notice.severity === 'error' || notice.severity === 'warning'" class="h-3.5 w-3.5 shrink-0" />
            <span>{{ notice.session_id ? `${agentName(notice.session_id)} · ` : '' }}{{ notice.title }}</span>
          </p>
          <p v-if="notice.description" class="mt-1 whitespace-pre-wrap break-words text-muted-foreground">
            {{ notice.description }}
          </p>
        </li>
      </ul>

      <details
        v-for="compaction in details.compactions"
        :key="compaction.id"
        class="rounded-md border border-border/70 bg-muted/20 p-2.5"
      >
        <summary class="flex cursor-pointer items-center justify-between gap-2 font-medium">
          <span>{{ t('chat.externalAgentDetails.compaction', 'Context compaction') }}</span>
          <span class="rounded-full border px-1.5 py-0.5 text-[10px] font-medium" :class="statusPillClass(compaction.status)">
            {{ t(`chat.externalAgentDetails.states.${compaction.status}`, compaction.status) }}
          </span>
        </summary>
        <pre v-if="compaction.summary" class="mt-2 whitespace-pre-wrap break-words font-sans text-muted-foreground">{{ compaction.summary }}</pre>
        <p v-if="compaction.error" class="mt-2 break-words text-destructive">{{ compaction.error }}</p>
      </details>

      <div
        v-for="(message, index) in details.messages"
        :key="message.id || index"
        class="rounded-r-md border-l-2 border-primary/40 bg-muted/15 py-1.5 pl-2.5 pr-2"
      >
        <p class="break-words font-medium text-foreground">{{ messageLabel(message) }}</p>
        <p class="mt-0.5 whitespace-pre-wrap break-words text-muted-foreground">{{ message.text }}</p>
      </div>

      <details
        v-for="terminal in details.terminals"
        :key="JSON.stringify([terminal.session_id, terminal.id])"
        class="rounded-md border border-border/70 bg-muted/20 p-2.5"
      >
        <summary class="flex cursor-pointer items-center justify-between gap-2 font-medium">
          <span class="min-w-0 truncate font-mono">
            {{ details.subagents?.[terminal.session_id] ? `${agentName(terminal.session_id)} · ` : '' }}
            {{ terminal.command || t('chat.externalAgentDetails.terminal', { id: terminal.id }) }}
          </span>
          <span
            class="shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] font-medium"
            :class="statusPillClass(terminal.exited ? (terminal.exit_code && terminal.exit_code !== 0 ? 'failed' : 'completed') : 'running')"
          >
            {{
              terminal.exit_code !== undefined
                ? t('chat.externalAgentDetails.exitCode', { code: terminal.exit_code })
                : t(terminal.exited ? 'chat.externalAgentDetails.states.completed' : 'chat.externalAgentDetails.states.running')
            }}
          </span>
        </summary>
        <div class="mt-2 space-y-1.5">
          <div class="flex items-center justify-between gap-2">
            <p v-if="terminal.cwd" class="min-w-0 flex-1 break-all font-mono text-[11px] text-muted-foreground">
              {{ terminal.cwd }}<span v-if="terminal.signal"> · {{ terminal.signal }}</span>
            </p>
            <IconButton
              v-if="terminal.output"
              size="sm"
              variant="ghost"
              class="h-6 w-6 shrink-0"
              :tooltip="t('common.copy')"
              :aria-label="t('common.copy')"
              @click="copyTerminalOutput(JSON.stringify([terminal.session_id, terminal.id]), terminal.output)"
            >
              <RiCheckLine
                v-if="copiedTerminalKey === JSON.stringify([terminal.session_id, terminal.id])"
                class="h-3.5 w-3.5 text-emerald-500"
              />
              <RiFileCopyLine v-else class="h-3.5 w-3.5" />
            </IconButton>
          </div>
          <pre
            v-if="terminal.output"
            class="max-h-56 overflow-auto rounded border border-border/60 bg-background/80 p-2 font-mono text-[11px] whitespace-pre"
          >{{ terminal.output }}</pre>
        </div>
      </details>

      <details
        v-for="[id, child] in children"
        :key="id"
        class="rounded-md border border-border/70 bg-muted/20 p-2.5"
      >
        <summary class="flex cursor-pointer items-center justify-between gap-2 font-medium">
          <span class="min-w-0 truncate">{{ child.title || id }}</span>
          <span
            v-if="child.state"
            class="shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] font-medium"
            :class="statusPillClass(child.state)"
          >
            {{ t(`chat.externalAgentDetails.states.${child.state}`, child.state) }}
          </span>
        </summary>
        <div class="mt-2 grid gap-1.5">
          <p v-if="child.description" class="break-words text-muted-foreground">{{ child.description }}</p>
          <p v-if="child.error" class="break-words text-destructive">{{ child.error }}</p>
          <p v-else-if="child.stop_reason" class="break-words text-muted-foreground">
            {{ t(`chat.externalAgentDetails.states.${child.stop_reason}`, child.stop_reason) }}
          </p>
          <p v-if="child.cwd" class="break-all font-mono text-[11px] text-muted-foreground">{{ child.cwd }}</p>
          <p v-for="directory in child.additional_directories" :key="directory" class="break-all font-mono text-[11px] text-muted-foreground">
            {{ directory }}
          </p>
          <div
            v-for="(message, index) in child.messages"
            :key="message.id || index"
            class="rounded-r-md border-l-2 border-border pl-2"
          >
            <p class="break-words font-medium">{{ messageLabel(message) }}</p>
            <p class="whitespace-pre-wrap break-words text-muted-foreground">{{ message.text }}</p>
          </div>
        </div>
      </details>

      <p
        v-if="!hasAnyContent"
        class="rounded-md border border-dashed border-border/70 bg-muted/15 px-3 py-5 text-center text-muted-foreground"
      >
        {{ t('chat.externalAgentDetails.emptyActivity') }}
      </p>
    </div>
  </SessionChip>
</template>
