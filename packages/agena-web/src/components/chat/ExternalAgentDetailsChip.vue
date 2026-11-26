<script setup lang="ts">
import { RiPulseLine } from '@remixicon/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import SessionChip from './SessionChip.vue'
import ExternalAgentWorkList from './ExternalAgentWorkList.vue'
import { useToastsStore } from '@/stores/toasts'
import type { ExternalAgentDetails, ExternalAgentMessage, ExternalAgentSuggestion } from '@/types/chat'

const props = defineProps<{ sessionId: string; label: string; details?: ExternalAgentDetails; suggestionEnabled: boolean }>()
const emit = defineEmits<{ suggestion: [suggestion: ExternalAgentSuggestion] }>()
const { locale, t } = useI18n()
const toasts = useToastsStore()
const open = ref(false)
const details = computed<ExternalAgentDetails>(() => props.details ?? {})
const children = computed(() => Object.entries(props.details?.subagents ?? {}))
const amount = computed(() => {
  const cost = props.details?.cost
  return cost ? `${new Intl.NumberFormat(locale.value, { maximumSignificantDigits: 10 }).format(cost.amount)} ${cost.currency}` : ''
})
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
</script>

<template>
  <SessionChip
    v-model:open="open"
    :title="`${label} · ${t('chat.externalAgentDetails.activity', 'Activity')}`"
    :label="amount || t('chat.externalAgentDetails.activity', 'Activity')"
    panel-class="w-[min(92vw,31rem)]"
  >
    <template #icon><RiPulseLine class="h-3 w-3" /></template>
    <div class="grid gap-2.5 text-xs">
      <ExternalAgentWorkList v-if="open" :session-id="sessionId" />
      <div v-if="details.suggestions?.length" class="grid gap-1.5">
        <p class="font-medium">{{ t('chat.externalAgentDetails.suggestions') }}</p>
        <button
          v-for="(suggestion, i) in details.suggestions"
          :key="i"
          type="button"
          class="rounded border border-border p-2 text-start hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
          :disabled="!suggestionEnabled"
          @click="useSuggestion(suggestion)"
        >
          <template v-for="(block, j) in suggestion.prompt" :key="j">
            <span v-if="block.type === 'text'" class="whitespace-pre-wrap break-words">{{ block.text }}</span>
            <span v-else class="block break-all font-mono text-muted-foreground">{{ block.name }} · {{ block.uri }}</span>
          </template>
        </button>
      </div>
      <div v-if="amount" class="flex justify-between gap-3 border-b border-border pb-2">
        <span class="text-muted-foreground">{{ t('chat.externalAgentDetails.cost', 'Agent-reported total') }}</span>
        <span class="font-mono">{{ amount }}</span>
      </div>
      <ul v-if="details.notices?.length" class="grid gap-2" aria-live="polite">
        <li v-for="(notice, index) in details.notices" :key="index" class="rounded border border-border p-2">
          <p class="break-words font-medium" :class="{ 'text-destructive': notice.severity === 'error' }">
            {{ notice.session_id ? `${agentName(notice.session_id)} · ` : '' }}{{ notice.title }}
          </p>
          <p v-if="notice.description" class="mt-0.5 whitespace-pre-wrap break-words text-muted-foreground">
            {{ notice.description }}
          </p>
        </li>
      </ul>
      <details v-for="compaction in details.compactions" :key="compaction.id" class="rounded border border-border p-2">
        <summary class="cursor-pointer font-medium">
          {{ t('chat.externalAgentDetails.compaction', 'Context compaction') }} ·
          {{ t(`chat.externalAgentDetails.states.${compaction.status}`, compaction.status) }}
        </summary>
        <pre v-if="compaction.summary" class="mt-1.5 whitespace-pre-wrap break-words font-sans">{{ compaction.summary }}</pre>
        <p v-if="compaction.error" class="mt-1.5 break-words text-destructive">{{ compaction.error }}</p>
      </details>
      <div v-for="(message, index) in details.messages" :key="message.id || index" class="border-l border-border pl-2">
        <p class="break-words font-medium">{{ messageLabel(message) }}</p>
        <p class="whitespace-pre-wrap break-words">{{ message.text }}</p>
      </div>
      <details
        v-for="terminal in details.terminals"
        :key="JSON.stringify([terminal.session_id, terminal.id])"
        class="rounded border border-border p-2"
      >
        <summary class="cursor-pointer break-words font-medium">
          {{ details.subagents?.[terminal.session_id] ? `${agentName(terminal.session_id)} · ` : '' }}
          {{ terminal.command || t('chat.externalAgentDetails.terminal', { id: terminal.id }) }}
        </summary>
        <p v-if="terminal.cwd" class="mt-1 break-all font-mono text-muted-foreground">{{ terminal.cwd }}</p>
        <p class="mt-1 text-muted-foreground">
          {{ terminal.exit_code !== undefined ? t('chat.externalAgentDetails.exitCode', { code: terminal.exit_code }) :
            t(terminal.exited ? 'chat.externalAgentDetails.states.completed' : 'chat.externalAgentDetails.states.running') }}
          <span v-if="terminal.signal"> · {{ terminal.signal }}</span>
        </p>
        <pre v-if="terminal.output" class="mt-1.5 max-h-56 overflow-auto whitespace-pre font-mono">{{ terminal.output }}</pre>
      </details>
      <details v-for="[id, child] in children" :key="id" class="rounded border border-border p-2">
        <summary class="cursor-pointer break-words font-medium">
          {{ child.title || id }}<span v-if="child.state" class="ml-2 text-muted-foreground">
            {{ t(`chat.externalAgentDetails.states.${child.state}`, child.state) }}
          </span>
        </summary>
        <div class="mt-1.5 grid gap-1.5">
          <p v-if="child.description" class="break-words text-muted-foreground">{{ child.description }}</p>
          <p v-if="child.error" class="break-words text-destructive">{{ child.error }}</p>
          <p v-else-if="child.stop_reason" class="break-words text-muted-foreground">
            {{ t(`chat.externalAgentDetails.states.${child.stop_reason}`, child.stop_reason) }}
          </p>
          <p v-if="child.cwd" class="break-all font-mono text-muted-foreground">{{ child.cwd }}</p>
          <p v-for="directory in child.additional_directories" :key="directory" class="break-all font-mono text-muted-foreground">{{ directory }}</p>
          <div v-for="(message, index) in child.messages" :key="message.id || index" class="border-l border-border pl-2">
            <p class="break-words font-medium">{{ messageLabel(message) }}</p>
            <p class="whitespace-pre-wrap break-words">{{ message.text }}</p>
          </div>
        </div>
      </details>
    </div>
  </SessionChip>
</template>
