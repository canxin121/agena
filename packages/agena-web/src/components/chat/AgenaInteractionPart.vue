<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiCheckLine, RiCloseLine, RiShieldKeyholeLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Button from '@/components/ui/Button.vue'
import type { JsonValue } from '@/types/json'
import {
  jsonArray,
  jsonRecord,
  stringValue,
  type InteractionPresentation,
  type OperationPermissionPresentation,
} from '@/pages/chat/transcriptPartPresentation'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'

const props = defineProps<{
  sessionId?: string | null
  interaction?: InteractionPresentation | null
  permission?: OperationPermissionPresentation | null
}>()

const chat = useChatStore()
const toasts = useToastsStore()
const { t } = useI18n()

const busy = ref(false)
const selected = ref<string[][]>([])
const customDrafts = ref<string[]>([])
const customOpen = ref<boolean[]>([])

const interaction = computed(() => props.interaction || null)
const permission = computed(() => props.permission || null)
const requestId = computed(() => interaction.value?.requestId || permission.value?.requestId || '')
const isPermission = computed(() => Boolean(permission.value))
const isPendingPermission = computed(() => permission.value?.pending === true)
const isReview = computed(() => (interaction.value?.kind || '').trim().toLowerCase() === 'review')
const questions = computed(() => interaction.value?.questions || [])
const hasBody = computed(() => Boolean(interaction.value?.bodyMarkdown?.trim()))
// Keep the Web layout in lockstep with the TUI: only a single, single-choice
// question with options is the compact plan-review decision surface. Other
// review-shaped requests still render as the continuous ask-user body so no
// question is silently dropped.
const isReviewDecision = computed(() => {
  const question = questions.value[0]
  return Boolean(
    isReview.value && questions.value.length === 1 && question && !question.multiple && question.options.length,
  )
})

function answersFromReply(reply: JsonValue | null, index: number, questionId: string): string[] {
  const answers = jsonRecord(jsonRecord(reply).answers)
  const byId = jsonArray(answers[questionId]).map(stringValue).filter(Boolean)
  if (byId.length || questionId === String(index)) return byId
  return jsonArray(answers[String(index)]).map(stringValue).filter(Boolean)
}

function hasExplicitCustomOption(label: string): boolean {
  const normalized = label.trim().toLowerCase()
  return normalized === 'custom' || normalized === 'type your own answer'
}

function ensureSlots() {
  const count = questions.value.length
  if (selected.value.length !== count) selected.value = questions.value.map(() => [])
  if (customDrafts.value.length !== count) customDrafts.value = questions.value.map(() => '')
  if (customOpen.value.length !== count) customOpen.value = questions.value.map(() => false)
}

function syncReplyState() {
  ensureSlots()
  const reply = interaction.value?.reply || null
  selected.value = questions.value.map((question, index) => {
    const values = answersFromReply(reply, index, question.questionId || String(index))
    return values.filter(
      (value) => !hasExplicitCustomOption(value) && question.options.some((option) => option.label === value),
    )
  })
  customDrafts.value = questions.value.map((question, index) => {
    const values = answersFromReply(reply, index, question.questionId || String(index))
    return values
      .filter((value) => hasExplicitCustomOption(value) || !question.options.some((option) => option.label === value))
      .join(', ')
  })
  customOpen.value = customDrafts.value.map((value) => Boolean(value))
}

watch(
  () => requestId.value,
  () => {
    selected.value = []
    customDrafts.value = []
    customOpen.value = []
    syncReplyState()
  },
  { immediate: true },
)

watch(
  () => [questions.value.length, interaction.value?.reply],
  () => {
    if (!interaction.value?.pending) syncReplyState()
    else ensureSlots()
  },
  { deep: true },
)

function isOptionChecked(index: number, label: string): boolean {
  return (selected.value[index] || []).includes(label)
}

function customAllowed(index: number): boolean {
  const question = questions.value[index]
  return Boolean(question?.allowCustom || question?.options.some((option) => hasExplicitCustomOption(option.label)))
}

function visibleOptions(index: number) {
  const question = questions.value[index]
  if (!question) return []
  // Older request producers sometimes included a literal "Custom" option in
  // addition to allow_custom. Treat that label as the canonical custom slot so
  // it does not appear twice in the Web transcript.
  return question.options.filter((option) => !(customAllowed(index) && hasExplicitCustomOption(option.label)))
}

function toggleOption(index: number, label: string) {
  const question = questions.value[index]
  if (!question) return
  if (hasExplicitCustomOption(label)) {
    customOpen.value[index] = !customOpen.value[index]
    if (!question.multiple) selected.value[index] = []
    return
  }
  const current = selected.value[index] || []
  if (question.multiple) {
    selected.value[index] = current.includes(label) ? current.filter((value) => value !== label) : [...current, label]
  } else {
    selected.value[index] = [label]
    customOpen.value[index] = false
    customDrafts.value[index] = ''
  }
}

function toggleCustom(index: number) {
  const question = questions.value[index]
  if (!question || !customAllowed(index)) return
  customOpen.value[index] = !customOpen.value[index]
  if (!question.multiple) selected.value[index] = []
}

function answerFor(index: number): string[] {
  const question = questions.value[index]
  if (!question) return []
  const options = (selected.value[index] || []).filter(
    (label) => !hasExplicitCustomOption(label) && question.options.some((option) => option.label === label),
  )
  const custom = customOpen.value[index] ? (customDrafts.value[index] || '').trim() : ''
  return custom ? [...options, custom] : options
}

const answerState = computed(() => questions.value.map((_, index) => answerFor(index)))
const canSubmit = computed(
  () =>
    Boolean(props.sessionId && requestId.value) &&
    !busy.value &&
    questions.value.length > 0 &&
    answerState.value.every((answer) => answer.length > 0),
)

async function replyPermission(reply: 'once' | 'always' | 'reject') {
  if (!props.sessionId || !requestId.value) return
  busy.value = true
  try {
    await chat.replyPermission(props.sessionId, requestId.value, reply)
    toasts.push(
      'success',
      reply === 'reject' ? t('chat.attention.toasts.permissionRejected') : t('chat.attention.toasts.permissionGranted'),
    )
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

async function submitAnswers() {
  if (!props.sessionId || !requestId.value) return
  const answers = answerState.value
  if (!answers.every((answer) => answer.length > 0)) {
    toasts.push('error', t('chat.attention.toasts.pleaseAnswerAllQuestions'))
    return
  }
  busy.value = true
  try {
    await chat.replyQuestion(props.sessionId, requestId.value, answers)
    toasts.push('success', t('chat.attention.toasts.answerSent'))
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

async function rejectAnswers() {
  if (!props.sessionId || !requestId.value) return
  busy.value = true
  try {
    await chat.rejectQuestion(props.sessionId, requestId.value)
    toasts.push('success', t('chat.attention.toasts.questionRejected'))
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <section
    v-if="interaction || permission"
    class="min-w-0 border-y border-border/55 py-3"
    data-transcript-interaction-part="true"
    :data-transcript-interaction-kind="isPermission ? 'permission' : isReviewDecision ? 'review' : 'ask-user'"
  >
    <template v-if="permission">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div class="text-xs font-semibold">{{ t('chat.attention.ui.permissionLabel') }}</div>
          <div class="mt-0.5 break-words font-mono text-[11px] text-foreground/90">{{ permission.action }}</div>
        </div>
        <span
          class="shrink-0 font-mono text-[10px]"
          :class="isPendingPermission ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'"
        >
          {{ permission.status }}
        </span>
      </div>
      <div v-if="permission.reason" class="mt-2 text-xs text-muted-foreground">{{ permission.reason }}</div>
      <div
        v-if="permission.explanation && permission.explanation !== permission.reason"
        class="mt-1 text-xs text-muted-foreground"
      >
        {{ permission.explanation }}
      </div>
      <div v-if="permission.provenance" class="mt-1 font-mono text-[10px] text-muted-foreground/75">
        {{ permission.provenance }}
      </div>
      <div v-if="permission.replyReason" class="mt-2 text-xs text-muted-foreground">
        <span class="font-semibold">{{ t('chat.attention.ui.replyReasonLabel') }}:</span>
        {{ permission.replyReason }}
      </div>
      <div v-if="isPendingPermission" class="mt-3 flex flex-wrap items-center gap-2">
        <Button size="sm" variant="ghost" :disabled="busy" @click="replyPermission('reject')">
          <RiCloseLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.rejectPermission') }}
        </Button>
        <Button size="sm" variant="outline" :disabled="busy" @click="replyPermission('once')">
          <RiCheckLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.allowOnce') }}
        </Button>
        <Button size="sm" variant="default" :disabled="busy" @click="replyPermission('always')">
          <RiShieldKeyholeLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.alwaysAllow') }}
        </Button>
      </div>
    </template>

    <template v-else-if="interaction">
      <div v-if="interaction.title && !isReviewDecision" class="mb-2 text-xs font-semibold">
        {{ interaction.title }}
      </div>
      <MarkdownRenderer
        v-if="interaction.bodyMarkdown"
        :content="interaction.bodyMarkdown"
        mode="markdown"
        :stream="false"
      />
      <div v-if="hasBody && questions.length" class="my-3 border-t border-border/60" aria-hidden="true" />

      <div v-if="isReviewDecision" class="space-y-2">
        <fieldset v-if="questions[0]" class="space-y-1">
          <legend v-if="questions[0].question && !hasBody" class="mb-2 text-xs font-medium">
            {{ questions[0].question }}
          </legend>
          <label
            v-for="option in visibleOptions(0)"
            :key="option.label"
            class="flex cursor-pointer items-start gap-2 rounded-md px-2 py-2 text-xs transition-colors hover:bg-muted/40"
            :class="isOptionChecked(0, option.label) ? 'bg-muted/55' : ''"
          >
            <input
              type="radio"
              :name="`review-${requestId}`"
              :checked="isOptionChecked(0, option.label)"
              :disabled="!interaction.pending || busy"
              class="mt-0.5 accent-primary"
              @change="toggleOption(0, option.label)"
            />
            <span class="min-w-0">
              <span class="font-medium">{{ option.label }}</span>
              <span v-if="option.description" class="ml-2 text-muted-foreground">{{ option.description }}</span>
            </span>
          </label>
          <button
            v-if="questions[0] && customAllowed(0) && (interaction.pending || customDrafts[0])"
            type="button"
            class="flex w-full items-start gap-2 rounded-md px-2 py-2 text-left text-xs transition-colors hover:bg-muted/40"
            :class="customOpen[0] ? 'bg-muted/55' : ''"
            :disabled="!interaction.pending || busy"
            @click="toggleCustom(0)"
          >
            <span class="mt-0.5 font-mono text-primary">{{ customOpen[0] ? '(x)' : '( )' }}</span>
            <span class="font-medium">{{ t('chat.attention.ui.custom') }}</span>
          </button>
        </fieldset>
        <textarea
          v-if="questions[0] && customAllowed(0) && customOpen[0]"
          v-model="customDrafts[0]"
          rows="3"
          :disabled="!interaction.pending || busy"
          class="w-full resize-y rounded-md border border-input bg-transparent px-2.5 py-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
          :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
        />
        <div v-if="!interaction.pending && answerFor(0).length" class="text-[11px] text-muted-foreground">
          <span class="font-semibold">{{ t('chat.attention.ui.answerLabel') }}:</span>
          {{ answerFor(0).join(', ') }}
        </div>
      </div>

      <div v-else class="space-y-4">
        <div v-if="!questions.length" class="text-xs text-muted-foreground">
          {{ t('chat.attention.ui.noQuestionsAvailable') }}
        </div>
        <section
          v-for="(question, questionIndex) in questions"
          :key="question.questionId || questionIndex"
          class="space-y-2"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                {{ question.header || `Q${questionIndex + 1}` }}
              </div>
              <div class="mt-1 text-xs font-medium">{{ question.question }}</div>
            </div>
            <span class="shrink-0 font-mono text-[10px] text-muted-foreground">
              {{ question.multiple ? t('chat.attention.ui.multiple') : t('chat.attention.ui.single') }}
            </span>
          </div>
          <div class="divide-y divide-border/35 border-y border-border/50">
            <label
              v-for="option in visibleOptions(questionIndex)"
              :key="option.label"
              class="flex cursor-pointer items-start gap-2 px-2 py-2 text-xs transition-colors hover:bg-muted/40"
              :class="isOptionChecked(questionIndex, option.label) ? 'bg-muted/55' : ''"
            >
              <input
                :type="question.multiple ? 'checkbox' : 'radio'"
                :name="`question-${requestId}-${questionIndex}`"
                :checked="isOptionChecked(questionIndex, option.label)"
                :disabled="!interaction.pending || busy"
                class="mt-0.5 accent-primary"
                @change="toggleOption(questionIndex, option.label)"
              />
              <span class="min-w-0">
                <span class="font-medium">{{ option.label }}</span>
                <span v-if="option.description" class="ml-2 text-muted-foreground">{{ option.description }}</span>
              </span>
            </label>
          </div>
          <button
            v-if="customAllowed(questionIndex) && (interaction.pending || customDrafts[questionIndex])"
            type="button"
            class="text-left text-[11px] text-primary hover:underline"
            :disabled="!interaction.pending || busy"
            @click="toggleCustom(questionIndex)"
          >
            {{ customOpen[questionIndex] ? t('chat.attention.ui.custom') : t('chat.attention.ui.typeYourOwnAnswer') }}
          </button>
          <textarea
            v-if="customAllowed(questionIndex) && customOpen[questionIndex]"
            v-model="customDrafts[questionIndex]"
            rows="2"
            :disabled="!interaction.pending || busy"
            class="w-full resize-y rounded-md border border-input bg-transparent px-2.5 py-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
          />
          <div v-if="!interaction.pending && answerFor(questionIndex).length" class="text-[11px] text-muted-foreground">
            <span class="font-semibold">{{ t('chat.attention.ui.answerLabel') }}:</span>
            {{ answerFor(questionIndex).join(', ') }}
          </div>
        </section>
      </div>

      <div v-if="interaction.pending" class="mt-3 flex flex-wrap items-center gap-2 border-t border-border/55 pt-3">
        <Button size="sm" variant="ghost" :disabled="busy" @click="rejectAnswers">
          <RiCloseLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.rejectQuestion') }}
        </Button>
        <Button size="sm" variant="default" :disabled="!canSubmit" @click="submitAnswers">
          <RiCheckLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.sendAnswers') }}
        </Button>
        <span v-if="!canSubmit" class="text-[10px] text-muted-foreground">
          {{ t('chat.attention.subtitle.answerAllToEnableSend') }}
        </span>
      </div>
      <div v-else class="mt-3 font-mono text-[10px] text-muted-foreground">
        {{ interaction.reply ? t('chat.attention.ui.answered') : t('chat.attention.ui.awaitingUserInput') }}
      </div>
    </template>
  </section>
</template>
