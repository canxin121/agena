<script setup lang="ts">
import { computed, nextTick, onMounted, onScopeDispose, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import AgenaTranscriptPart from './AgenaTranscriptPart.vue'
import Button from '@/components/ui/Button.vue'
import { useChatStore } from '@/stores/chat'
import { useSettingsStore } from '@/stores/settings'
import { createTranscriptProjector, transcriptProjectionLabels } from '@/pages/chat/transcriptProjection'
import type { TranscriptDisplayPart } from './messageList.types'

const props = defineProps<{ sessionId: string }>()
const { t } = useI18n()
const chat = useChatStore()
const settings = useSettingsStore()
const transcript = ref<HTMLElement>()
const limit = ref(8)
const paging = ref(false)
const choices = reactive(new Map<string, boolean>())
const history = computed(() => chat.getSessionHistory(props.sessionId))
const error = computed(() => chat.historyErrorBySession[props.sessionId])
const messages = computed(() => chat.getMessagesForSession(props.sessionId))
const project = createTranscriptProjector(() => ({
  showReasoning: settings.data?.showReasoningTraces !== false,
  labels: transcriptProjectionLabels(t),
}))
const blocks = computed(() => project(messages.value.slice(-limit.value)))
const expanded = (part: TranscriptDisplayPart) => choices.get(part.key) ?? part.defaultExpanded
const body = () => transcript.value?.closest('[data-session-section-body]')

// Share the child's live transcript and interaction state with any other pane that has it open.
onScopeDispose(chat.retainSession(props.sessionId))
void chat.refreshMessages(props.sessionId, { silent: true })

async function loadEarlier() {
  if (paging.value) return
  paging.value = true
  const region = body()
  const height = region?.scrollHeight ?? 0
  try {
    if (messages.value.length <= limit.value) await chat.loadOlderMessages(props.sessionId)
    limit.value += 8
    await nextTick()
    if (region) region.scrollTop += region.scrollHeight - height
  } finally {
    paging.value = false
  }
}

onMounted(() => {
  const region = body()
  if (region) region.scrollTop = region.scrollHeight
})
watch(blocks, async () => {
  const region = body()
  if (!region || paging.value || region.scrollHeight - region.scrollTop - region.clientHeight > 24) return
  await nextTick()
  region.scrollTop = region.scrollHeight
})
</script>

<template>
  <div ref="transcript" class="space-y-2" data-agent-messages>
    <p v-if="error" role="alert" class="text-xs text-destructive">{{ error }}</p>
    <p
      v-else-if="!blocks.length && history.userMessageCount == null"
      role="status"
      class="text-xs text-muted-foreground"
    >
      {{ t('common.loading') }}
    </p>
    <p v-else-if="!blocks.length" class="text-xs text-muted-foreground">{{ t('bot.conversations.empty') }}</p>
    <Button
      v-if="history.userMessageCount != null && (messages.length > limit || !history.exhausted)"
      size="sm"
      variant="ghost"
      :disabled="paging || history.loading"
      @click="loadEarlier"
    >
      {{ t(paging || history.loading ? 'chat.messages.loadingOlder' : 'chat.messages.loadOlder') }}
    </Button>
    <article
      v-for="block in blocks"
      :key="block.key"
      class="min-w-0 border-l-2 border-border/70 pl-2"
      :class="{ 'border-primary/45': block.message.info.role === 'user' }"
    >
      <div class="mb-0.5 text-[11px] font-medium text-muted-foreground">
        {{
          t(
            `chat.roles.${block.message.info.role === 'user' ? 'user' : block.message.info.role === 'assistant' ? 'assistant' : 'system'}`,
          )
        }}
      </div>
      <AgenaTranscriptPart
        v-for="part in block.displayParts"
        :key="part.key"
        :part="part"
        :session-id="sessionId"
        :expanded="expanded(part)"
        :collapse-signal="0"
        @toggle="choices.set(part.key, !expanded(part))"
      />
    </article>
  </div>
</template>
