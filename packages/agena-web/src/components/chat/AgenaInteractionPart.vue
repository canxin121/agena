<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
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
const rootEl = ref<HTMLElement | null>(null)
const activeControlKey = ref('')

const interaction = computed(() => props.interaction || null)
const permission = computed(() => props.permission || null)
const requestId = computed(() => interaction.value?.requestId || permission.value?.requestId || '')
const isPermission = computed(() => Boolean(permission.value))
const isPendingPermission = computed(() => permission.value?.pending === true)
const isReview = computed(() => (interaction.value?.kind || '').trim().toLowerCase() === 'review')
const questions = computed(() => interaction.value?.questions || [])
const hasBody = computed(() => Boolean(interaction.value?.bodyMarkdown?.trim()))
const isPendingInteraction = computed(() => Boolean(permission.value?.pending || interaction.value?.pending))
const hasKeyboardControls = computed(() => isPendingInteraction.value)
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

function optionControlKey(index: number, label: string): string {
  return `question:${index}:option:${label}`
}

function customControlKey(index: number): string {
  return `question:${index}:custom`
}

function textareaControlKey(index: number): string {
  return `question:${index}:textarea`
}

function controlTabIndex(key: string): number {
  return activeControlKey.value === key ? 0 : -1
}

function interactionControls(): HTMLElement[] {
  const root = rootEl.value
  if (!root) return []
  return Array.from(root.querySelectorAll<HTMLElement>('[data-interaction-control="true"]')).filter(
    (element) => !element.hasAttribute('disabled') && element.getAttribute('aria-disabled') !== 'true',
  )
}

function controlKey(element: Element | null | undefined): string {
  return element?.getAttribute('data-interaction-control-key') || ''
}

function controlKind(element: Element | null | undefined): string {
  return element?.getAttribute('data-interaction-control-kind') || ''
}

function controlQuestionIndex(element: Element | null | undefined): number {
  const raw = element?.getAttribute('data-interaction-question-index') || ''
  const index = Number(raw)
  return Number.isInteger(index) && index >= 0 ? index : -1
}

function focusControlElement(element: HTMLElement | null | undefined) {
  if (!element) return
  const key = controlKey(element)
  if (key) activeControlKey.value = key
  element.focus({ preventScroll: true })
}

function focusControlByKey(key: string) {
  if (!key) return
  activeControlKey.value = key
  nextTick(() => {
    const target = interactionControls().find((element) => controlKey(element) === key)
    if (target) focusControlElement(target)
  })
}

function focusFirstControl(reverse = false) {
  nextTick(() => {
    const controls = interactionControls()
    if (controls.length) {
      focusControlElement(reverse ? controls.at(-1) : controls[0])
      return
    }
    rootEl.value?.focus({ preventScroll: true })
  })
}

function markControlFocus(event: FocusEvent) {
  const target = event.currentTarget
  if (target instanceof Element) {
    const key = controlKey(target)
    if (key) activeControlKey.value = key
  }
}

function questionControls(index: number, includeTextarea = false): HTMLElement[] {
  return interactionControls().filter((element) => {
    if (controlQuestionIndex(element) !== index) return false
    const kind = controlKind(element)
    if (kind === 'option' || kind === 'custom') return true
    return includeTextarea && kind === 'textarea'
  })
}

function currentQuestionIndex(target: Element | null | undefined): number {
  const targetIndex = controlQuestionIndex(target)
  if (targetIndex >= 0) return targetIndex
  if (isReviewDecision.value) return 0
  const firstUnanswered = answerState.value.findIndex((answer) => answer.length === 0)
  return firstUnanswered >= 0 ? firstUnanswered : questions.value.length ? 0 : -1
}

function focusQuestionEntry(index: number, reverse = false) {
  const controls = questionControls(index, true)
  focusControlElement(reverse ? controls.at(-1) : controls[0])
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
  if (customOpen.value[index]) {
    focusControlByKey(textareaControlKey(index))
  } else {
    nextTick(() => focusQuestionEntry(index))
  }
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

function openCustomEditor(index: number) {
  const question = questions.value[index]
  if (!question || !customAllowed(index) || !interaction.value?.pending || busy.value) return
  customOpen.value[index] = true
  if (!question.multiple) selected.value[index] = []
  focusControlByKey(textareaControlKey(index))
}

function focusNextIncompleteQuestion(fromIndex: number) {
  const nextIndex = questions.value.findIndex((_, index) => index > fromIndex && answerFor(index).length === 0)
  if (nextIndex >= 0) {
    nextTick(() => focusQuestionEntry(nextIndex))
    return
  }
  const firstIndex = questions.value.findIndex((_, index) => answerFor(index).length === 0)
  if (firstIndex >= 0) nextTick(() => focusQuestionEntry(firstIndex))
}

async function confirmCurrentOption(target: Element | null) {
  const index = currentQuestionIndex(target)
  if (index < 0) return
  const question = questions.value[index]
  if (!question) return

  const kind = controlKind(target)
  if (kind === 'option') {
    const label = target?.getAttribute('data-interaction-control-value') || ''
    if (label && !isOptionChecked(index, label)) toggleOption(index, label)
  }

  if (isReviewDecision.value || answerState.value.every((answer) => answer.length > 0)) {
    await submitAnswers()
  } else {
    focusNextIncompleteQuestion(index)
  }
}

function moveArrowFocus(target: Element | null, direction: 1 | -1) {
  const kind = controlKind(target)
  if (kind === 'textarea') return

  if (isPermission.value) {
    const controls = interactionControls().filter((element) => controlKind(element) === 'permission')
    const currentIndex = Math.max(0, controls.indexOf(target as HTMLElement))
    const nextIndex = (currentIndex + direction + controls.length) % controls.length
    if (controls.length) {
      focusControlElement(controls[nextIndex])
      return
    }
  }

  if (kind === 'option' || kind === 'custom') {
    const index = currentQuestionIndex(target)
    const controls = questionControls(index)
    const currentIndex = Math.max(0, controls.indexOf(target as HTMLElement))
    if (controls.length) {
      if (direction > 0 && currentIndex === controls.length - 1 && index < questions.value.length - 1) {
        focusQuestionEntry(index + 1)
        return
      }
      if (direction < 0 && currentIndex === 0 && index > 0) {
        focusQuestionEntry(index - 1, true)
        return
      }
      const nextIndex = (currentIndex + direction + controls.length) % controls.length
      focusControlElement(controls[nextIndex])
    }
    return
  }

  const controls = interactionControls()
  const currentIndex = Math.max(0, controls.indexOf(target as HTMLElement))
  if (controls.length) {
    const nextIndex = (currentIndex + direction + controls.length) % controls.length
    focusControlElement(controls[nextIndex])
  }
}

function handleTab(event: KeyboardEvent, target: HTMLElement) {
  const controls = interactionControls()
  if (!controls.length) return
  const current = target.closest<HTMLElement>('[data-interaction-control="true"]')
  if (!current) {
    event.preventDefault()
    focusFirstControl(event.shiftKey)
    return
  }
  const currentIndex = controls.indexOf(current)
  const nextIndex = currentIndex + (event.shiftKey ? -1 : 1)
  if (nextIndex < 0 || nextIndex >= controls.length) return
  event.preventDefault()
  focusControlElement(controls[nextIndex])
}

function clearCustomDraft(target: Element | null) {
  const index = currentQuestionIndex(target)
  if (index < 0 || !customAllowed(index)) return
  customDrafts.value[index] = ''
}

async function cancelInteraction() {
  if (busy.value || !isPendingInteraction.value) return
  if (isPermission.value) {
    await replyPermission('reject')
    return
  }
  await rejectAnswers()
}

function handleEscape(target: Element | null) {
  const index = currentQuestionIndex(target)
  if (index >= 0 && customOpen.value[index]) {
    customOpen.value[index] = false
    nextTick(() => focusQuestionEntry(index))
    return
  }
  const active = document.activeElement
  if (active instanceof HTMLElement && rootEl.value?.contains(active)) active.blur()
}

async function handleKeydown(event: KeyboardEvent) {
  const target = event.target instanceof HTMLElement ? event.target : rootEl.value
  if (!target) return
  const control = target.closest<HTMLElement>('[data-interaction-control="true"]')
  const kind = controlKind(control)
  const key = event.key
  const lowerKey = key.toLowerCase()

  if (event.ctrlKey && lowerKey === 'x') {
    event.preventDefault()
    await cancelInteraction()
    return
  }
  if (event.ctrlKey && lowerKey === 'd') {
    event.preventDefault()
    clearCustomDraft(control)
    return
  }
  if (key === 'Escape') {
    event.preventDefault()
    handleEscape(control)
    return
  }

  const isTextEditor =
    target instanceof HTMLTextAreaElement || (target instanceof HTMLInputElement && target.type === 'text')
  if (isTextEditor) {
    // Match the TUI custom editor: Enter submits non-empty feedback and an
    // empty Enter closes the editor. Shift+Enter keeps the Web textarea's
    // native multiline behavior; Ctrl+Enter is an additional explicit submit
    // chord for users who prefer it.
    if (key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      const index = currentQuestionIndex(control)
      if (index >= 0 && (customDrafts.value[index] || '').trim()) {
        if (isReviewDecision.value || answerState.value.every((answer) => answer.length > 0)) {
          await submitAnswers()
        } else {
          // TUI commits the current custom answer and advances through the
          // question flow instead of attempting a partial submission.
          focusNextIncompleteQuestion(index)
        }
      } else if (index >= 0) {
        customOpen.value[index] = false
        nextTick(() => focusQuestionEntry(index))
      }
    }
    return
  }

  if (key === 'Tab') {
    handleTab(event, target)
    return
  }
  if (key === 'ArrowUp') {
    event.preventDefault()
    moveArrowFocus(control, -1)
    return
  }
  if (key === 'ArrowDown') {
    event.preventDefault()
    moveArrowFocus(control, 1)
    return
  }
  if ((lowerKey === 'k' || lowerKey === 'j') && isReview.value && !event.altKey && !event.ctrlKey && !event.metaKey) {
    event.preventDefault()
    moveArrowFocus(control, lowerKey === 'k' ? -1 : 1)
    return
  }
  if (lowerKey === 'e' && !event.altKey && !event.metaKey) {
    const index = currentQuestionIndex(control)
    if (index >= 0 && customAllowed(index)) {
      event.preventDefault()
      openCustomEditor(index)
    }
    return
  }
  if (key === ' ' || key === 'Spacebar') {
    if (kind === 'option') {
      event.preventDefault()
      const index = currentQuestionIndex(control)
      const label = control?.getAttribute('data-interaction-control-value') || ''
      if (index >= 0 && label) toggleOption(index, label)
    } else if (kind === 'custom') {
      event.preventDefault()
      const index = currentQuestionIndex(control)
      if (index >= 0) toggleCustom(index)
    }
    return
  }
  if (key === 'Enter') {
    if (kind === 'option') {
      event.preventDefault()
      await confirmCurrentOption(control)
    } else if (kind === 'custom') {
      event.preventDefault()
      const index = currentQuestionIndex(control)
      if (index >= 0) toggleCustom(index)
    } else if (!control) {
      event.preventDefault()
      await confirmCurrentOption(null)
    }
  }
}

function focusPendingInteraction(force = false) {
  if (!isPendingInteraction.value) return
  nextTick(() => {
    const root = rootEl.value
    if (!root) return
    const current = document.activeElement
    const currentIsControl =
      current instanceof HTMLElement &&
      root.contains(current) &&
      current.getAttribute('data-interaction-control') === 'true'
    if (!force && currentIsControl) {
      const key = controlKey(current)
      if (key) activeControlKey.value = key
      return
    }
    const controls = interactionControls()
    if (controls.length) {
      focusControlElement(controls[0])
    } else {
      root.focus({ preventScroll: true })
    }
  })
}

watch(
  () => [requestId.value, isPendingInteraction.value, questions.value.length, isPermission.value],
  () => {
    activeControlKey.value = ''
    focusPendingInteraction(true)
  },
  { immediate: true },
)

onMounted(focusPendingInteraction)
</script>

<template>
  <section
    v-if="interaction || permission"
    ref="rootEl"
    class="min-w-0 border-y border-border/55 py-3 outline-none focus-visible:ring-1 focus-visible:ring-ring/60"
    data-transcript-interaction-part="true"
    data-transcript-chrome="true"
    :tabindex="hasKeyboardControls ? -1 : 0"
    :data-transcript-interaction-kind="isPermission ? 'permission' : isReviewDecision ? 'review' : 'ask-user'"
    @keydown.stop="handleKeydown"
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
        <Button
          size="sm"
          variant="ghost"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="permission"
          data-interaction-control-key="permission:reject"
          :tabindex="controlTabIndex('permission:reject')"
          @focus="markControlFocus"
          @click="replyPermission('reject')"
        >
          <RiCloseLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.rejectPermission') }}
        </Button>
        <Button
          size="sm"
          variant="outline"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="permission"
          data-interaction-control-key="permission:once"
          :tabindex="controlTabIndex('permission:once')"
          @focus="markControlFocus"
          @click="replyPermission('once')"
        >
          <RiCheckLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.allowOnce') }}
        </Button>
        <Button
          size="sm"
          variant="default"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="permission"
          data-interaction-control-key="permission:always"
          :tabindex="controlTabIndex('permission:always')"
          @focus="markControlFocus"
          @click="replyPermission('always')"
        >
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
              data-interaction-control="true"
              data-interaction-control-kind="option"
              :data-interaction-control-key="optionControlKey(0, option.label)"
              :data-interaction-control-value="option.label"
              data-interaction-question-index="0"
              :tabindex="controlTabIndex(optionControlKey(0, option.label))"
              @focus="markControlFocus"
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
            data-interaction-control="true"
            data-interaction-control-kind="custom"
            :data-interaction-control-key="customControlKey(0)"
            data-interaction-question-index="0"
            :tabindex="controlTabIndex(customControlKey(0))"
            @focus="markControlFocus"
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
          data-interaction-control="true"
          data-interaction-control-kind="textarea"
          :data-interaction-control-key="textareaControlKey(0)"
          data-interaction-question-index="0"
          :tabindex="controlTabIndex(textareaControlKey(0))"
          @focus="markControlFocus"
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
                data-interaction-control="true"
                data-interaction-control-kind="option"
                :data-interaction-control-key="optionControlKey(questionIndex, option.label)"
                :data-interaction-control-value="option.label"
                :data-interaction-question-index="questionIndex"
                :tabindex="controlTabIndex(optionControlKey(questionIndex, option.label))"
                @focus="markControlFocus"
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
            data-interaction-control="true"
            data-interaction-control-kind="custom"
            :data-interaction-control-key="customControlKey(questionIndex)"
            :data-interaction-question-index="questionIndex"
            :tabindex="controlTabIndex(customControlKey(questionIndex))"
            @focus="markControlFocus"
            @click="toggleCustom(questionIndex)"
          >
            {{ customOpen[questionIndex] ? t('chat.attention.ui.custom') : t('chat.attention.ui.typeYourOwnAnswer') }}
          </button>
          <textarea
            v-if="customAllowed(questionIndex) && customOpen[questionIndex]"
            v-model="customDrafts[questionIndex]"
            rows="2"
            :disabled="!interaction.pending || busy"
            data-interaction-control="true"
            data-interaction-control-kind="textarea"
            :data-interaction-control-key="textareaControlKey(questionIndex)"
            :data-interaction-question-index="questionIndex"
            :tabindex="controlTabIndex(textareaControlKey(questionIndex))"
            @focus="markControlFocus"
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
        <Button
          size="sm"
          variant="ghost"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:reject"
          :tabindex="controlTabIndex('action:reject')"
          @focus="markControlFocus"
          @click="rejectAnswers"
        >
          <RiCloseLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.rejectQuestion') }}
        </Button>
        <Button
          size="sm"
          variant="default"
          :disabled="!canSubmit"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:submit"
          :tabindex="controlTabIndex('action:submit')"
          @focus="markControlFocus"
          @click="submitAnswers"
        >
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
