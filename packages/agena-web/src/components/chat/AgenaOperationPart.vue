<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AttentionPanel from '@/components/chat/AttentionPanel.vue'
import AgenaOperationBlock from '@/components/chat/AgenaOperationBlock.vue'
import type { AttentionLike, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import {
  operationPresentation,
  partStatusPresentation,
  prettyJson,
  jsonRecord,
  jsonArray,
  stringValue,
  type InteractionPresentation,
  attentionRequestId,
} from '@/pages/chat/transcriptPartPresentation'

const props = defineProps<{
  part: TranscriptDisplayPart
  expanded: boolean
  collapseSignal: number
  sessionId?: string | null
  attention?: AttentionLike
}>()

const emit = defineEmits<{
  (event: 'toggle'): void
  (event: 'select'): void
}>()

const operation = computed(() => operationPresentation(props.part))
const status = computed(() => partStatusPresentation(props.part.status))
const inputExpanded = ref(false)
const outputExpanded = ref(false)
const permissionsExpanded = ref(false)

watch(
  () => props.collapseSignal,
  () => {
    inputExpanded.value = false
    outputExpanded.value = false
    permissionsExpanded.value = false
  },
)

const primaryOutput = computed(() => operation.value.humanMarkdown || operation.value.modelOutput)
const primaryOutputIsMarkdown = computed(() => Boolean(operation.value.humanMarkdown))
const primaryInteraction = computed(() => operation.value.userInputs[0] || null)
const headlineTitle = computed(() =>
  primaryInteraction.value ? operation.value.toolName || operation.value.title : operation.value.title,
)
const headlineSummary = computed(
  () => primaryInteraction.value?.questions[0]?.question || primaryInteraction.value?.title || operation.value.summary,
)
const hasOutput = computed(() => {
  const value = operation.value
  return Boolean(
    primaryOutput.value ||
    value.structured !== null ||
    value.displaySections.length ||
    value.blocks.length ||
    value.attachments.length,
  )
})
const activeAttentionRequestId = computed(() => attentionRequestId(props.attention?.payload))

function interactionOwnsAttention(interaction: InteractionPresentation): boolean {
  return (
    Boolean(interaction.pending && interaction.requestId) && interaction.requestId === activeAttentionRequestId.value
  )
}

const pendingPermissionOwnsAttention = computed(() =>
  operation.value.permissions.some(
    (permission) =>
      permission.status === 'Awaiting user approval' &&
      Boolean(permission.requestId) &&
      permission.requestId === activeAttentionRequestId.value,
  ),
)

function toggleOuter() {
  emit('select')
  emit('toggle')
}

function interactionAnswers(interaction: InteractionPresentation, questionIndex: number): string[] {
  const answers = jsonRecord(jsonRecord(interaction.reply).answers)
  return jsonArray(answers[String(questionIndex)]).map(stringValue).filter(Boolean)
}
</script>

<template>
  <div class="min-w-0">
    <button
      type="button"
      class="group/headline flex w-full min-w-0 items-baseline gap-2 py-1 text-left outline-none"
      :aria-expanded="expanded"
      data-transcript-vim-toggle="true"
      @click="toggleOuter"
      @focus="$emit('select')"
    >
      <span class="w-3 shrink-0 text-center font-mono text-xs text-muted-foreground" aria-hidden="true">{{
        expanded ? '▾' : '▸'
      }}</span>
      <span
        class="w-3 shrink-0 text-center font-mono text-xs"
        :class="{
          'text-primary': status.tone === 'pending',
          'text-emerald-600 dark:text-emerald-400': status.tone === 'success',
          'text-amber-600 dark:text-amber-400': status.tone === 'warning',
          'text-rose-600 dark:text-rose-400': status.tone === 'danger',
          'text-muted-foreground': status.tone === 'muted',
          'animate-spin': status.spinning,
        }"
        :title="status.label"
        aria-hidden="true"
        >{{ status.icon }}</span
      >
      <span class="min-w-0 flex-1 truncate text-[13px] font-semibold">{{ headlineTitle }}</span>
      <span v-if="!expanded && headlineSummary" class="min-w-0 max-w-[55%] truncate text-xs text-muted-foreground"
        >· {{ headlineSummary }}</span
      >
      <span v-if="operation.durationMs !== null" class="shrink-0 font-mono text-[10px] text-muted-foreground/70">
        {{ operation.durationMs }}ms
      </span>
    </button>

    <div v-if="expanded" class="ml-5 border-l border-border/60 pl-4 pb-1">
      <div v-if="operation.userInputs.length" class="space-y-4 py-1 text-sm">
        <section v-for="interaction in operation.userInputs" :key="interaction.requestId || interaction.title">
          <AttentionPanel
            v-if="interactionOwnsAttention(interaction) && attention && sessionId"
            :kind="attention.kind"
            :session-id="sessionId"
            :payload="attention.payload"
            inline
          />
          <template v-else>
            <MarkdownRenderer
              v-if="interaction.bodyMarkdown"
              :content="interaction.bodyMarkdown"
              mode="markdown"
              :stream="false"
            />
            <div
              v-for="(question, questionIndex) in interaction.questions"
              :key="`${question.question}:${questionIndex}`"
              class="mt-2"
            >
              <div v-if="question.header" class="text-[10px] font-semibold uppercase text-muted-foreground">
                {{ question.header }}
              </div>
              <div class="font-medium">{{ question.question }}</div>
              <div class="mt-1 border-y border-border/50">
                <div v-for="option in question.options" :key="option.label" class="py-1.5 text-xs">
                  <span class="font-mono text-primary">{{
                    interactionAnswers(interaction, questionIndex).includes(option.label) ? '(x)' : '( )'
                  }}</span>
                  <span class="ml-2 font-medium">{{ option.label }}</span>
                  <span v-if="option.description" class="ml-2 text-muted-foreground">{{ option.description }}</span>
                </div>
              </div>
              <div
                v-for="custom in interactionAnswers(interaction, questionIndex).filter(
                  (answer) => !question.options.some((option) => option.label === answer),
                )"
                :key="custom"
                class="mt-1 border-l border-primary/40 pl-2 text-xs"
              >
                {{ custom }}
              </div>
            </div>
            <div v-if="interaction.pending" class="mt-2 font-mono text-[11px] text-amber-600 dark:text-amber-400">
              Awaiting user input
            </div>
          </template>
        </section>
      </div>

      <section v-else-if="operation.permissions.length" class="py-1">
        <AttentionPanel
          v-if="pendingPermissionOwnsAttention && attention && sessionId"
          :kind="attention.kind"
          :session-id="sessionId"
          :payload="attention.payload"
          inline
        />
        <template v-else>
          <button
            type="button"
            class="flex items-center gap-2 py-1 text-xs font-semibold text-primary outline-none"
            :aria-expanded="permissionsExpanded"
            @click="permissionsExpanded = !permissionsExpanded"
          >
            <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
              permissionsExpanded ? '▾' : '▸'
            }}</span>
            Permissions
          </button>
          <div v-if="permissionsExpanded" class="divide-y divide-border/50 pl-5 pt-1 text-xs">
            <div
              v-for="permission in operation.permissions"
              :key="`${permission.action}:${permission.status}`"
              class="py-2"
            >
              <div class="font-medium">{{ permission.status }} · {{ permission.action }}</div>
              <div v-if="permission.reason" class="mt-1 text-muted-foreground">Request: {{ permission.reason }}</div>
              <div
                v-if="permission.explanation && permission.explanation !== permission.reason"
                class="mt-1 text-muted-foreground"
              >
                Policy: {{ permission.explanation }}
              </div>
              <div
                v-if="permission.replyReason && permission.replyReason !== permission.reason"
                class="mt-1 text-muted-foreground"
              >
                Reply: {{ permission.replyReason }}
              </div>
              <div v-if="permission.provenance" class="mt-1 font-mono text-[10px] text-muted-foreground/75">
                {{ permission.provenance }}
              </div>
            </div>
          </div>
        </template>
      </section>

      <section v-if="!operation.userInputs.length && operation.error" class="py-1.5">
        <div class="text-xs font-semibold text-rose-600 dark:text-rose-400">› Error</div>
        <pre
          class="mt-1 whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-rose-700 dark:text-rose-300"
          >{{ operation.error }}</pre
        >
      </section>

      <section v-if="!operation.userInputs.length && operation.input !== null" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 py-1 text-xs font-semibold text-primary outline-none"
          :aria-expanded="inputExpanded"
          @click="inputExpanded = !inputExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            inputExpanded ? '▾' : '▸'
          }}</span>
          Input
        </button>
        <div v-if="inputExpanded" class="pl-5 pt-1">
          <MarkdownRenderer
            v-if="operation.inputMarkdown"
            :content="operation.inputMarkdown"
            mode="markdown"
            :stream="false"
          />
          <CodeBlock v-else :code="prettyJson(operation.input)" lang="json" compact />
        </div>
      </section>

      <section v-if="!operation.userInputs.length && hasOutput" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 py-1 text-xs font-semibold text-primary outline-none"
          :aria-expanded="outputExpanded"
          @click="outputExpanded = !outputExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            outputExpanded ? '▾' : '▸'
          }}</span>
          Output
        </button>

        <div v-if="outputExpanded" class="space-y-3 pl-5 pt-1">
          <MarkdownRenderer
            v-if="primaryOutput && primaryOutputIsMarkdown"
            :content="primaryOutput"
            mode="markdown"
            :stream="false"
          />
          <pre
            v-else-if="primaryOutput"
            class="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-relaxed"
            >{{ primaryOutput }}</pre
          >

          <section v-for="section in operation.displaySections" :key="`${section.title}:${section.text}`">
            <div class="mb-1 text-xs font-semibold text-primary">› {{ section.title }}</div>
            <MarkdownRenderer :content="section.text" mode="markdown" :stream="false" />
          </section>

          <AgenaOperationBlock
            v-for="(block, index) in operation.blocks"
            :key="String(block.id || `${block.type || block.kind || 'block'}:${index}`)"
            :block="block"
          />

          <div v-if="operation.attachments.length" class="border-y border-border/50 py-1 font-mono text-xs">
            <a
              v-for="attachment in operation.attachments"
              :key="attachment.key"
              :href="attachment.url || undefined"
              :target="attachment.url ? '_blank' : undefined"
              :rel="attachment.url ? 'noopener noreferrer' : undefined"
              class="flex min-w-0 items-center gap-2 py-1 text-foreground hover:text-primary"
            >
              <span aria-hidden="true">›</span>
              <span class="min-w-0 flex-1 truncate">{{ attachment.label }}</span>
              <span v-if="attachment.mime" class="text-[10px] text-muted-foreground">{{ attachment.mime }}</span>
            </a>
          </div>

          <CodeBlock
            v-if="operation.structured !== null && !primaryOutput && !operation.blocks.length"
            :code="prettyJson(operation.structured)"
            lang="json"
            compact
          />
        </div>
      </section>
    </div>
  </div>
</template>
