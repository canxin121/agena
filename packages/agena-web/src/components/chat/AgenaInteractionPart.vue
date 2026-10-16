<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import { RiCheckLine, RiCloseLine, RiShieldKeyholeLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Button from '@/components/ui/Button.vue'
import { isImeComposing } from '@/lib/imeKeyboard'
import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'
import { interactionText } from '@/pages/chat/interactionText'
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

const rootEl = ref<HTMLElement | null>(null)
const activeControlKey = ref('')
const instanceId = useId()
const hintId = `${instanceId}-interaction-hint`
const displayText = (value: string) => interactionText(interaction.value?.kind || '', value, t)

const requestId = computed(() => props.interaction?.requestId || props.permission?.requestId || '')
const pendingRequest = computed(() =>
  chat.getPendingInteraction(props.sessionId || '', requestId.value, props.permission ? 'permission' : 'user_input'),
)
const interaction = computed(() =>
  props.interaction
    ? { ...props.interaction, pending: props.interaction.pending && Boolean(pendingRequest.value) }
    : null,
)
const permission = computed(() =>
  props.permission ? { ...props.permission, pending: props.permission.pending && Boolean(pendingRequest.value) } : null,
)
const unavailableStatus = computed(() =>
  t(
    chat.hasInteractionSnapshot(props.sessionId || '')
      ? 'chat.attention.ui.requestNoLongerPending'
      : 'chat.attention.ui.checkingRequestStatus',
  ),
)
// Attention can precede the durable owner's page. Keep the user's work by
// request identity when that owner arrives or the view remounts.
const draft = computed(() => chat.getInteractionDraft(props.sessionId || '', requestId.value))
const busy = computed(() => draft.value.busy)
const selected = computed({
  get: () => (interaction.value?.pending ? draft.value.selected : replyState.value.selected),
  set: (value) => {
    draft.value.selected = value
  },
})
const customDrafts = computed({
  get: () => (interaction.value?.pending ? draft.value.customDrafts : replyState.value.customDrafts),
  set: (value) => {
    draft.value.customDrafts = value
  },
})
const customOpen = computed({
  get: () => (interaction.value?.pending ? draft.value.customOpen : replyState.value.customOpen),
  set: (value) => {
    draft.value.customOpen = value
  },
})
const questionPage = computed({
  get: () => draft.value.questionPage,
  set: (value) => {
    draft.value.questionPage = value
  },
})
const isPermission = computed(() => Boolean(permission.value))
const isPendingPermission = computed(() => permission.value?.pending === true)
const isReview = computed(() => (interaction.value?.kind || '').trim().toLowerCase() === 'review')
const questions = computed(() => interaction.value?.questions || [])
const hasBody = computed(() => Boolean(interaction.value?.bodyMarkdown?.trim()))
const isPendingInteraction = computed(() => Boolean(permission.value?.pending || interaction.value?.pending))
const permissionStatusLabel = computed(() => {
  if (props.permission?.pending && !permission.value?.pending) return unavailableStatus.value
  const status = permission.value?.status || ''
  switch (status) {
    case 'Allowed once':
      return t('chat.attention.ui.permissionStatus.allowedOnce')
    case 'Allowed persistently':
      return t('chat.attention.ui.permissionStatus.allowedPersistently')
    case 'Approved automatically':
      return t('chat.attention.ui.permissionStatus.approvedAutomatically')
    case 'Denied once':
      return t('chat.attention.ui.permissionStatus.deniedOnce')
    case 'Denied persistently':
      return t('chat.attention.ui.permissionStatus.deniedPersistently')
    case 'Awaiting user approval':
      return t('chat.attention.ui.permissionStatus.awaitingApproval')
  }
  if (status.startsWith('Replied (') && status.endsWith(')')) {
    return t('chat.attention.ui.permissionStatus.replied', { kind: status.slice(9, -1) })
  }
  return status
})
// Keep the Web layout in lockstep with the TUI: only a single, single-choice
// question with options is the compact plan-review decision surface. Other
// review-shaped requests still render as the ask-user flow so no
// question is silently dropped.
const isReviewDecision = computed(() => {
  const question = questions.value[0]
  return Boolean(
    isReview.value && questions.value.length === 1 && question && !question.multiple && question.options.length,
  )
})

const isQuestionFlow = computed(() => Boolean(interaction.value?.pending && !isReviewDecision.value))
const isSummaryPage = computed(() => isQuestionFlow.value && questionPage.value === questions.value.length)
const visibleQuestions = computed(() =>
  questions.value
    .map((question, questionIndex) => ({ question, questionIndex }))
    .filter(({ questionIndex }) => !isQuestionFlow.value || questionIndex === questionPage.value),
)

function goToQuestionPage(index: number) {
  if (!isQuestionFlow.value || busy.value) return
  questionPage.value = Math.max(0, Math.min(index, questions.value.length))
  activeControlKey.value = ''
  focusFirstControl()
}

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
  if (selected.value.length !== count) selected.value = questions.value.map((_, index) => selected.value[index] || [])
  if (customDrafts.value.length !== count)
    customDrafts.value = questions.value.map((_, index) => customDrafts.value[index] || '')
  if (customOpen.value.length !== count)
    customOpen.value = questions.value.map((_, index) => customOpen.value[index] || false)
}

// History is a read-only projection of the committed reply. Keep local drafts
// separate so terminalizing an unanswered request neither displays a draft as
// an answer nor destroys the user's unsubmitted work.
const replyState = computed(() => {
  const reply = interaction.value?.reply || null
  const selected = questions.value.map((question, index) => {
    const values = answersFromReply(reply, index, question.questionId || String(index))
    return values.filter(
      (value) => !hasExplicitCustomOption(value) && question.options.some((option) => option.label === value),
    )
  })
  const customDrafts = questions.value.map((question, index) => {
    const values = answersFromReply(reply, index, question.questionId || String(index))
    return values
      .filter((value) => hasExplicitCustomOption(value) || !question.options.some((option) => option.label === value))
      .join(', ')
  })
  return { selected, customDrafts, customOpen: customDrafts.map(Boolean) }
})

watch(
  () => [requestId.value, questions.value.length, interaction.value?.pending],
  () => {
    if (interaction.value?.pending) ensureSlots()
  },
  { immediate: true },
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

function interactionControls(): HTMLElement[] {
  const root = rootEl.value
  if (!root) return []
  return Array.from(root.querySelectorAll<HTMLElement>('[data-interaction-control="true"]')).filter(
    (element) => !element.matches(':disabled, [aria-disabled="true"]') && isVisibleKeyboardTarget(element),
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
  if (!raw) return -1
  const index = Number(raw)
  return Number.isInteger(index) && index >= 0 ? index : -1
}

function focusControlElement(element: HTMLElement | null | undefined) {
  if (!element) return
  const key = controlKey(element)
  if (key) activeControlKey.value = key
  element.focus({ preventScroll: true })
  element.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}

function scheduleControlFocus(resolve: () => HTMLElement | null | undefined) {
  const root = rootEl.value
  const owner = requestId.value
  const origin = document.activeElement
  nextTick(() => {
    if (!root || root !== rootEl.value || owner !== requestId.value || !isVisibleKeyboardTarget(root)) return
    if (hasOpenKeyboardOverlay()) return
    const active = document.activeElement
    if (active !== origin && active !== document.body && !root.contains(active)) return
    const target = resolve()
    if (target) focusControlElement(target)
    else root.focus({ preventScroll: true })
  })
}

function focusControlByKey(key: string) {
  if (!key) return
  activeControlKey.value = key
  scheduleControlFocus(() => interactionControls().find((element) => controlKey(element) === key))
}

function focusFirstControl(reverse = false) {
  scheduleControlFocus(() => {
    const controls = interactionControls()
    return reverse ? controls.at(-1) : controls[0]
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
  if (isQuestionFlow.value) return isSummaryPage.value ? -1 : questionPage.value
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
  if (!question || !interaction.value?.pending || busy.value) return
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
  if (!question || !customAllowed(index) || !interaction.value?.pending || busy.value) return
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

function retainReplyFocus(): string | null {
  const root = rootEl.value
  if (!root?.contains(document.activeElement)) return null
  const key = activeControlKey.value
  root.focus({ preventScroll: true })
  return key
}

function restoreReplyFocus(key: string | null, replied: boolean) {
  if (key === null) return
  nextTick(() => {
    if (document.activeElement !== rootEl.value || hasOpenKeyboardOverlay()) return
    if (replied || !isPendingInteraction.value) returnToTranscript()
    else if (key) focusControlByKey(key)
    else focusFirstControl()
  })
}

async function replyPermission(reply: 'once' | 'always' | 'reject') {
  if (!props.sessionId || !requestId.value || busy.value || !isPendingPermission.value) return
  const focusKey = retainReplyFocus()
  let replied = false
  const pendingDraft = draft.value
  pendingDraft.busy = true
  try {
    replied = await chat.replyPermission(props.sessionId, requestId.value, reply)
    if (!replied) return
    toasts.push(
      'success',
      reply === 'reject' ? t('chat.attention.toasts.permissionRejected') : t('chat.attention.toasts.permissionGranted'),
    )
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    pendingDraft.busy = false
    restoreReplyFocus(focusKey, replied)
  }
}

async function submitAnswers() {
  if (!props.sessionId || !requestId.value || busy.value || !interaction.value?.pending) return
  if (isQuestionFlow.value && !isSummaryPage.value) {
    goToQuestionPage(questions.value.length)
    return
  }
  const answers = answerState.value
  if (!answers.length || !answers.every((answer) => answer.length > 0)) {
    toasts.push('error', t('chat.attention.toasts.pleaseAnswerAllQuestions'))
    goToQuestionPage(answers.findIndex((answer) => !answer.length))
    return
  }
  const focusKey = retainReplyFocus()
  let replied = false
  const pendingDraft = draft.value
  pendingDraft.busy = true
  try {
    replied = await chat.replyQuestion(props.sessionId, requestId.value, answers)
    if (!replied) return
    toasts.push('success', t('chat.attention.toasts.answerSent'))
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    pendingDraft.busy = false
    restoreReplyFocus(focusKey, replied)
  }
}

async function rejectAnswers() {
  if (!props.sessionId || !requestId.value || busy.value || !interaction.value?.pending) return
  const focusKey = retainReplyFocus()
  let replied = false
  const pendingDraft = draft.value
  pendingDraft.busy = true
  try {
    replied = await chat.rejectQuestion(props.sessionId, requestId.value)
    if (!replied) return
    toasts.push('success', t('chat.attention.toasts.questionRejected'))
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    pendingDraft.busy = false
    restoreReplyFocus(focusKey, replied)
  }
}

function openCustomEditor(index: number) {
  const question = questions.value[index]
  if (!question || !customAllowed(index) || !interaction.value?.pending || busy.value) return
  customOpen.value[index] = true
  if (!question.multiple) selected.value[index] = []
  focusControlByKey(textareaControlKey(index))
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

  if (isReviewDecision.value) {
    await submitAnswers()
  } else {
    goToQuestionPage(index + 1)
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
      if (
        !isQuestionFlow.value &&
        direction > 0 &&
        currentIndex === controls.length - 1 &&
        index < questions.value.length - 1
      ) {
        focusQuestionEntry(index + 1)
        return
      }
      if (!isQuestionFlow.value && direction < 0 && currentIndex === 0 && index > 0) {
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

function returnToTranscript() {
  const root = rootEl.value
  if (!root || !isVisibleKeyboardTarget(root)) return
  const part = root.closest<HTMLElement>('[data-transcript-node="part"]')
  const pane = root.closest<HTMLElement>('[data-workspace-pane-window], [data-keyboard-region], main')
  const target =
    part?.querySelector<HTMLElement>('[data-transcript-vim-toggle="true"]') ||
    part ||
    Array.from(pane?.querySelectorAll<HTMLElement>('[data-chat-input="true"]') || []).find(isVisibleKeyboardTarget) ||
    root
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}

function handleEscape(target: Element | null) {
  const index = currentQuestionIndex(target)
  if (isPendingInteraction.value && !busy.value && index >= 0 && customOpen.value[index]) {
    customOpen.value[index] = false
    focusControlByKey(customControlKey(index))
    return
  }
  returnToTranscript()
}

function consumeKey(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
}

async function handleKeydown(event: KeyboardEvent) {
  if (event.defaultPrevented || isImeComposing(event)) return
  const target = event.target instanceof HTMLElement ? event.target : rootEl.value
  if (!target) return
  const control = target.closest<HTMLElement>('[data-interaction-control="true"]')
  const kind = controlKind(control)
  const key = event.key
  const lowerKey = key.toLowerCase()
  const isTextEditor = Boolean(target.closest('textarea, input[type="text"], [contenteditable="true"]'))

  // Native Tab order includes the custom editor, navigation and action buttons.
  // Let unhandled shortcuts reach the app (including focus-composer shortcuts).
  if (key === 'Tab') return
  if (key === 'Escape' && !event.ctrlKey && !event.altKey && !event.metaKey) {
    consumeKey(event)
    handleEscape(control)
    return
  }
  // Markdown links, code blocks and their controls own their keyboard actions.
  if (!control && target !== rootEl.value) return
  if (!isPendingInteraction.value || busy.value) return
  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && key === 'Enter') {
    if (!isPermission.value) {
      consumeKey(event)
      await submitAnswers()
    }
    return
  }
  if (isTextEditor) {
    // Preserve native editing, cut/paste, selection and IME confirmation.
    if (key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) {
      consumeKey(event)
      const index = currentQuestionIndex(control)
      if (index >= 0 && (customDrafts.value[index] || '').trim()) {
        if (isReviewDecision.value) await submitAnswers()
        else goToQuestionPage(index + 1)
      } else if (index >= 0) {
        customOpen.value[index] = false
        focusControlByKey(customControlKey(index))
      }
    }
    return
  }
  if (event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && lowerKey === 'x') {
    consumeKey(event)
    await cancelInteraction()
    return
  }
  if (event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && lowerKey === 'd') {
    consumeKey(event)
    clearCustomDraft(control)
    return
  }
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return
  if (isQuestionFlow.value && (key === 'ArrowLeft' || key === 'ArrowRight')) {
    consumeKey(event)
    goToQuestionPage(questionPage.value + (key === 'ArrowRight' ? 1 : -1))
    return
  }
  if (
    key === 'ArrowUp' ||
    key === 'ArrowDown' ||
    (isReview.value && (key === 'ArrowLeft' || key === 'ArrowRight' || lowerKey === 'k' || lowerKey === 'j'))
  ) {
    consumeKey(event)
    moveArrowFocus(control, key === 'ArrowUp' || key === 'ArrowLeft' || lowerKey === 'k' ? -1 : 1)
    return
  }
  if (key === 'Home' || key === 'End') {
    consumeKey(event)
    const index = currentQuestionIndex(control)
    const controls = kind === 'option' || kind === 'custom' ? questionControls(index) : interactionControls()
    focusControlElement(key === 'Home' ? controls[0] : controls.at(-1))
    return
  }
  if (lowerKey === 'e') {
    const index = currentQuestionIndex(control)
    if (index >= 0 && customAllowed(index)) {
      consumeKey(event)
      openCustomEditor(index)
    }
    return
  }
  if (key === ' ' || key === 'Spacebar') {
    if (kind === 'option') {
      consumeKey(event)
      const index = currentQuestionIndex(control)
      const label = control?.getAttribute('data-interaction-control-value') || ''
      if (index >= 0 && label) toggleOption(index, label)
    } else if (kind === 'custom') {
      consumeKey(event)
      const index = currentQuestionIndex(control)
      if (index >= 0) toggleCustom(index)
    }
    return
  }
  if (key === 'Enter') {
    if (kind === 'option') {
      consumeKey(event)
      await confirmCurrentOption(control)
    } else if (kind === 'custom') {
      consumeKey(event)
      const index = currentQuestionIndex(control)
      if (index >= 0) toggleCustom(index)
    } else if (!control && target === rootEl.value) {
      consumeKey(event)
      focusFirstControl()
    }
  }
}

// A request can finish in another client. Move focus before its controls are
// disabled; mounting/streaming requests never take focus from another pane.
watch(isPendingInteraction, (pending, previous) => {
  if (!pending && previous && rootEl.value?.contains(document.activeElement)) returnToTranscript()
})

// Attention can be replaced by its durable operation while the user types.
// Restore only the focus that this instance owned, within the same visible pane.
onBeforeUnmount(() => {
  const root = rootEl.value
  if (!root?.contains(document.activeElement)) return
  const scope = root.closest<HTMLElement>('[data-workspace-pane-window], main') || root.parentElement
  const owner = requestId.value
  const session = props.sessionId || ''
  const key = activeControlKey.value
  const headline = root
    .closest('[data-transcript-node="part"]')
    ?.querySelector<HTMLElement>('[data-transcript-vim-toggle="true"]')
  nextTick(() => {
    if (!scope || !isVisibleKeyboardTarget(scope) || hasOpenKeyboardOverlay()) return
    if (document.activeElement !== document.body) return
    const replacement = Array.from(
      scope.querySelectorAll<HTMLElement>('[data-transcript-interaction-part="true"]'),
    ).find(
      (element) =>
        element.dataset.interactionRequestId === owner &&
        element.dataset.interactionSessionId === session &&
        isVisibleKeyboardTarget(element),
    )
    const target = Array.from(
      replacement?.querySelectorAll<HTMLElement>('[data-interaction-control="true"]') || [],
    ).find((element) => controlKey(element) === key && !element.matches(':disabled, [aria-disabled="true"]'))
    if (target) target.focus({ preventScroll: true })
    else if (headline && isVisibleKeyboardTarget(headline)) headline.focus({ preventScroll: true })
  })
})
</script>

<template>
  <section
    v-if="interaction || permission"
    ref="rootEl"
    class="min-w-0 border-y border-border/55 py-1 outline-none focus-visible:ring-1 focus-visible:ring-ring/60"
    data-transcript-interaction-part="true"
    :data-interaction-request-id="requestId"
    :data-interaction-session-id="sessionId || ''"
    data-transcript-chrome="true"
    :tabindex="isPendingInteraction ? -1 : 0"
    :aria-label="
      permission
        ? t('chat.attention.ui.permissionLabel')
        : displayText(interaction?.title || t('chat.attention.title.question'))
    "
    :aria-describedby="isPendingInteraction ? hintId : undefined"
    :aria-busy="busy"
    :data-transcript-interaction-kind="isPermission ? 'permission' : isReviewDecision ? 'review' : 'ask-user'"
    @keydown="handleKeydown"
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
          {{ permissionStatusLabel }}
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
      <div v-if="isPendingPermission" class="mt-1.5 flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="ghost"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="permission"
          data-interaction-control-key="permission:reject"
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
          @focus="markControlFocus"
          @click="replyPermission('always')"
        >
          <RiShieldKeyholeLine class="mr-1 h-3.5 w-3.5" />
          {{ t('chat.attention.ui.alwaysAllow') }}
        </Button>
      </div>
    </template>

    <template v-else-if="interaction">
      <div
        v-if="props.interaction?.pending && !interaction.pending"
        class="mb-2 text-xs text-muted-foreground"
        role="status"
      >
        {{ unavailableStatus }}
      </div>
      <div v-if="interaction.title" class="mb-2 text-xs font-semibold">
        {{ displayText(interaction.title) }}
      </div>
      <MarkdownRenderer
        v-if="interaction.bodyMarkdown && (!isQuestionFlow || questionPage === 0)"
        :content="interaction.bodyMarkdown"
        mode="markdown"
        :stream="false"
      />
      <div
        v-if="hasBody && questions.length && (!isQuestionFlow || questionPage === 0)"
        class="my-1.5 border-t border-border/60"
        aria-hidden="true"
      />

      <div
        v-if="isQuestionFlow"
        class="mb-1.5 text-xs font-medium text-muted-foreground"
        aria-live="polite"
        data-interaction-page
      >
        {{
          isSummaryPage
            ? t('chat.attention.ui.reviewAnswers')
            : t('chat.attention.ui.questionPager', { current: questionPage + 1, total: questions.length })
        }}
      </div>

      <div v-if="isReviewDecision" class="space-y-1.5">
        <fieldset
          v-if="questions[0]"
          class="space-y-1"
          :aria-label="displayText(questions[0].question)"
          :aria-describedby="isPendingInteraction ? hintId : undefined"
        >
          <legend v-if="questions[0].question && !hasBody" class="mb-2 text-xs font-medium">
            {{ displayText(questions[0].question) }}
          </legend>
          <label
            v-for="option in visibleOptions(0)"
            :key="option.label"
            class="flex cursor-pointer items-start gap-2 rounded-md px-2 py-1 text-xs transition-colors hover:bg-muted/40"
            :class="isOptionChecked(0, option.label) ? 'bg-muted/55' : ''"
          >
            <input
              type="radio"
              :name="`${instanceId}-review-${requestId}`"
              :checked="isOptionChecked(0, option.label)"
              :disabled="!interaction.pending || busy"
              tabindex="0"
              data-interaction-control="true"
              data-interaction-control-kind="option"
              :data-interaction-control-key="optionControlKey(0, option.label)"
              :data-interaction-control-value="option.label"
              data-interaction-question-index="0"
              @focus="markControlFocus"
              class="mt-0.5 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
              @change="toggleOption(0, option.label)"
            />
            <span class="min-w-0">
              <span class="font-medium">{{ displayText(option.label) }}</span>
              <span v-if="option.description" class="ml-2 text-muted-foreground">{{
                displayText(option.description)
              }}</span>
            </span>
          </label>
          <button
            tabindex="0"
            v-if="questions[0] && customAllowed(0) && (interaction.pending || customDrafts[0])"
            type="button"
            class="flex w-full items-start gap-2 rounded-md px-2 py-1 text-left text-xs transition-colors hover:bg-muted/40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
            :class="customOpen[0] ? 'bg-muted/55' : ''"
            :aria-expanded="Boolean(customOpen[0])"
            :disabled="!interaction.pending || busy"
            data-interaction-control="true"
            data-interaction-control-kind="custom"
            :data-interaction-control-key="customControlKey(0)"
            data-interaction-question-index="0"
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
          tabindex="0"
          data-interaction-control="true"
          data-interaction-control-kind="textarea"
          :data-interaction-control-key="textareaControlKey(0)"
          data-interaction-question-index="0"
          @focus="markControlFocus"
          class="w-full resize-y rounded-md border border-input bg-transparent px-2.5 py-1 text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
          :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
          :aria-label="t('chat.attention.ui.customAnswerFor', { question: displayText(questions[0]?.question || '') })"
        />
        <div v-if="!interaction.pending && answerFor(0).length" class="text-[11px] text-muted-foreground">
          <span class="font-semibold">{{ t('chat.attention.ui.answerLabel') }}:</span>
          {{ answerFor(0).map(displayText).join(', ') }}
        </div>
      </div>

      <div v-else-if="isSummaryPage" class="space-y-1.5" data-interaction-summary>
        <div
          v-for="(question, questionIndex) in questions"
          :key="question.questionId || questionIndex"
          class="flex items-start justify-between gap-3 rounded-md border border-border/60 p-2"
        >
          <div class="min-w-0 text-xs">
            <div class="font-medium break-words">{{ displayText(question.header || question.question) }}</div>
            <div
              class="mt-1 whitespace-pre-wrap break-words"
              :class="answerState[questionIndex]?.length ? 'text-foreground' : 'text-muted-foreground'"
            >
              {{ answerState[questionIndex]?.map(displayText).join(', ') || t('chat.attention.ui.unanswered') }}
            </div>
          </div>
          <Button
            size="sm"
            variant="ghost"
            :disabled="busy"
            data-interaction-control="true"
            data-interaction-control-kind="summary"
            :data-interaction-control-key="`summary:${questionIndex}`"
            :aria-label="`${t('chat.attention.ui.editAnswer')}: ${displayText(question.header || question.question)}`"
            @focus="markControlFocus"
            @click="goToQuestionPage(questionIndex)"
          >
            {{ t('chat.attention.ui.editAnswer') }}
          </Button>
        </div>
      </div>

      <div v-else class="space-y-1.5">
        <div v-if="!questions.length" class="text-xs text-muted-foreground">
          {{ t('chat.attention.ui.noQuestionsAvailable') }}
        </div>
        <section
          v-for="{ question, questionIndex } in visibleQuestions"
          :key="question.questionId || questionIndex"
          role="group"
          :aria-label="displayText(question.question)"
          :aria-describedby="isPendingInteraction ? hintId : undefined"
          class="space-y-1.5"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                {{
                  question.header
                    ? displayText(question.header)
                    : t('chat.attention.ui.numberedQuestion', { count: questionIndex + 1 })
                }}
              </div>
              <div class="mt-1 text-xs font-medium">{{ displayText(question.question) }}</div>
            </div>
            <span class="shrink-0 font-mono text-[10px] text-muted-foreground">
              {{ question.multiple ? t('chat.attention.ui.multiple') : t('chat.attention.ui.single') }}
            </span>
          </div>
          <div class="divide-y divide-border/35 border-y border-border/50">
            <label
              v-for="option in visibleOptions(questionIndex)"
              :key="option.label"
              class="flex cursor-pointer items-start gap-2 px-2 py-1 text-xs transition-colors hover:bg-muted/40"
              :class="isOptionChecked(questionIndex, option.label) ? 'bg-muted/55' : ''"
            >
              <input
                :type="question.multiple ? 'checkbox' : 'radio'"
                :name="`${instanceId}-question-${requestId}-${questionIndex}`"
                :checked="isOptionChecked(questionIndex, option.label)"
                :disabled="!interaction.pending || busy"
                tabindex="0"
                data-interaction-control="true"
                data-interaction-control-kind="option"
                :data-interaction-control-key="optionControlKey(questionIndex, option.label)"
                :data-interaction-control-value="option.label"
                :data-interaction-question-index="questionIndex"
                @focus="markControlFocus"
                class="mt-0.5 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                @change="toggleOption(questionIndex, option.label)"
              />
              <span class="min-w-0">
                <span class="font-medium">{{ displayText(option.label) }}</span>
                <span v-if="option.description" class="ml-2 text-muted-foreground">{{
                  displayText(option.description)
                }}</span>
              </span>
            </label>
          </div>
          <button
            tabindex="0"
            v-if="customAllowed(questionIndex) && (interaction.pending || customDrafts[questionIndex])"
            type="button"
            class="rounded text-left text-[11px] text-primary hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
            :aria-expanded="Boolean(customOpen[questionIndex])"
            :disabled="!interaction.pending || busy"
            data-interaction-control="true"
            data-interaction-control-kind="custom"
            :data-interaction-control-key="customControlKey(questionIndex)"
            :data-interaction-question-index="questionIndex"
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
            tabindex="0"
            data-interaction-control="true"
            data-interaction-control-kind="textarea"
            :data-interaction-control-key="textareaControlKey(questionIndex)"
            :data-interaction-question-index="questionIndex"
            @focus="markControlFocus"
            class="w-full resize-y rounded-md border border-input bg-transparent px-2.5 py-1 text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
            :aria-label="t('chat.attention.ui.customAnswerFor', { question: displayText(question.question) })"
          />
          <div v-if="!interaction.pending && answerFor(questionIndex).length" class="text-[11px] text-muted-foreground">
            <span class="font-semibold">{{ t('chat.attention.ui.answerLabel') }}:</span>
            {{ answerFor(questionIndex).map(displayText).join(', ') }}
          </div>
        </section>
      </div>

      <div
        v-if="isQuestionFlow"
        class="mt-1.5 flex items-center justify-between gap-2 border-t border-border/55 pt-1.5"
      >
        <Button
          size="sm"
          variant="ghost"
          :disabled="busy || questionPage === 0"
          data-interaction-control="true"
          data-interaction-control-kind="navigation"
          data-interaction-control-key="page:previous"
          @focus="markControlFocus"
          @click="goToQuestionPage(questionPage - 1)"
        >
          ‹ {{ t('chat.attention.ui.previousQuestion') }}
        </Button>
        <Button
          v-if="!isSummaryPage"
          size="sm"
          variant="outline"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="navigation"
          data-interaction-control-key="page:next"
          @focus="markControlFocus"
          @click="goToQuestionPage(questionPage + 1)"
        >
          {{
            questionPage + 1 === questions.length
              ? t('chat.attention.ui.reviewAnswers')
              : t('chat.attention.ui.nextQuestion')
          }}
          ›
        </Button>
      </div>
      <div
        v-if="interaction.pending && (isReviewDecision || isSummaryPage)"
        class="mt-1.5 flex flex-wrap items-center gap-2 border-t border-border/55 pt-1.5"
      >
        <Button
          size="sm"
          variant="ghost"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:reject"
          @focus="markControlFocus"
          @click="rejectAnswers"
        >
          <RiCloseLine class="mr-1 h-3.5 w-3.5" />
          {{ t('common.cancel') }}
        </Button>
        <Button
          size="sm"
          variant="default"
          :disabled="!canSubmit"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:submit"
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
      <div v-else-if="!interaction.pending" class="mt-1.5 font-mono text-[10px] text-muted-foreground">
        {{ interaction.reply ? t('chat.attention.ui.answered') : t('chat.attention.ui.unanswered') }}
      </div>
    </template>
    <p v-if="isPendingInteraction" :id="hintId" class="mt-2 text-[10px] text-muted-foreground">
      {{
        t(
          isPermission
            ? 'chat.attention.ui.permissionKeyboardHint'
            : isReviewDecision
              ? 'chat.attention.ui.reviewKeyboardHint'
              : 'chat.attention.ui.questionKeyboardHint',
        )
      }}
    </p>
  </section>
</template>
