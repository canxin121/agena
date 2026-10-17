<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import {
  RiArrowLeftSLine,
  RiArrowRightSLine,
  RiChat3Line,
  RiCheckLine,
  RiCloseCircleLine,
  RiEditLine,
  RiLoader4Line,
  RiPauseCircleLine,
  RiPlayCircleLine,
  RiShieldKeyholeLine,
  RiStopCircleLine,
  RiTimeLine,
} from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Button from '@/components/ui/Button.vue'
import { isImeComposing } from '@/lib/imeKeyboard'
import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'
import { interactionText } from '@/pages/chat/interactionText'
import {
  actionCursorNeighbour,
  transcriptActionRect,
  type ActionCursorDirection,
} from '@/pages/chat/transcriptActionCursor'
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
    isReview.value &&
    questions.value.length === 1 &&
    question &&
    !question.multiple &&
    question.options.length &&
    !question.options.some((option) => option.custom),
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

function moveQuestionPage(delta: number) {
  if (!isQuestionFlow.value || !questions.value.length) return
  if (isSummaryPage.value && delta > 0) return
  const page = Math.max(0, Math.min(questionPage.value + delta, questions.value.length - 1))
  if (page !== questionPage.value) goToQuestionPage(page)
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
    return values.filter((value) => question.options.some((option) => !option.custom && option.label === value))
  })
  const customDrafts = questions.value.map((question, index) => {
    const values = answersFromReply(reply, index, question.questionId || String(index))
    return values
      .filter((value) => !question.options.some((option) => !option.custom && option.label === value))
      .join(', ')
  })
  return { selected, customDrafts, customOpen: customDrafts.map(Boolean) }
})

const replyCancelled = computed(() => {
  const reply = jsonRecord(interaction.value?.reply)
  return reply.kind === 'cancel' || reply.cancelled === true
})
const replyStatusLabel = computed(() =>
  replyCancelled.value
    ? t('chat.partStatus.cancelled')
    : t(interaction.value?.reply ? 'chat.attention.ui.answered' : 'chat.attention.ui.unanswered'),
)
const committedAnswers = computed(() =>
  questions.value.map((question, index) => ({
    question,
    answers: answersFromReply(interaction.value?.reply || null, index, question.questionId || String(index)).map(
      (value) => {
        const option = question.options.find((option) => !option.custom && option.label === value)
        return { value, description: option?.description || '', custom: !option }
      },
    ),
  })),
)

// Protocol values determine the icon, never a translated label or its position.
// Unknown producer-defined decisions retain a neutral action affordance.
function reviewOptionAppearance(label: string) {
  switch (label) {
    case 'Approve with autorun on':
      return { icon: RiPlayCircleLine, tone: 'accent' }
    case 'Approve with autorun off':
    case 'Approve requested status with auto-continue off':
      return { icon: RiPauseCircleLine, tone: 'accent' }
    case 'Approve':
    case 'Approve requested status':
      return { icon: RiCheckLine, tone: 'accent' }
    case 'Keep in planning':
      return { icon: RiEditLine, tone: 'neutral' }
    case 'Reject':
      return { icon: RiCloseCircleLine, tone: 'destructive' }
    case 'Cancel plan':
      return { icon: RiStopCircleLine, tone: 'destructive' }
    default:
      return { icon: RiArrowRightSLine, tone: 'neutral' }
  }
}

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
  return Boolean(question?.allowCustom || question?.options.some((option) => option.custom))
}

function customOption(index: number) {
  return questionOptions(index).find((option) => option.custom)
}

function isCustomSelected(index: number) {
  const option = customOption(index)
  return Boolean(option && isOptionChecked(index, option.label))
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
  return questionOptions(index).filter((option) => !option.custom)
}

function questionOptions(index: number) {
  const question = questions.value[index]
  if (!question) return []
  // Older producers included a literal Custom choice alongside allow_custom.
  // An explicit option type takes precedence over that legacy label convention.
  const legacyCustomIndex =
    question.allowCustom && !question.options.some((option) => option.custom)
      ? question.options.findIndex((option) => hasExplicitCustomOption(option.label))
      : -1
  const options = question.options.map((option, optionIndex) => ({
    ...option,
    custom: Boolean(option.custom || optionIndex === legacyCustomIndex),
    synthetic: false,
  }))
  if (question.allowCustom && !options.some((option) => option.custom))
    options.push({ label: 'Custom', description: '', custom: true, synthetic: true })
  return options
}

function toggleOption(index: number, label: string) {
  const question = questions.value[index]
  if (!question || !interaction.value?.pending || busy.value) return
  const option = questionOptions(index).find((option) => option.label === label)
  if (!option) return
  const current = selected.value[index] || []
  if (question.multiple) {
    selected.value[index] = current.includes(label) ? current.filter((value) => value !== label) : [...current, label]
  } else {
    selected.value[index] = [label]
  }
  if (option.custom) {
    customOpen.value[index] = isCustomSelected(index)
    if (customOpen.value[index]) focusControlByKey(textareaControlKey(index))
  } else if (!question.multiple) customOpen.value[index] = false
}

function toggleCustom(index: number) {
  const question = questions.value[index]
  if (!question || !customAllowed(index) || !interaction.value?.pending || busy.value) return
  if (!isReviewDecision.value) {
    const option = customOption(index)
    if (option) toggleOption(index, option.label)
    return
  }
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
  const options = (selected.value[index] || []).filter((label) =>
    questionOptions(index).some((option) => !option.custom && option.label === label),
  )
  const custom = (isReviewDecision.value ? customOpen.value[index] : isCustomSelected(index))
    ? (customDrafts.value[index] || '').trim()
    : ''
  return custom ? [...options, custom] : options
}

function questionAnswered(index: number) {
  return answerFor(index).length > 0 && (!isCustomSelected(index) || Boolean((customDrafts.value[index] || '').trim()))
}

const answerState = computed(() => questions.value.map((_, index) => answerFor(index)))
const canSubmit = computed(
  () =>
    Boolean(props.sessionId && requestId.value) &&
    !busy.value &&
    questions.value.length > 0 &&
    questions.value.every((_, index) => questionAnswered(index)),
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
  const answers = answerState.value
  if (!answers.length || !questions.value.every((_, index) => questionAnswered(index))) {
    toasts.push('error', t('chat.attention.toasts.pleaseAnswerAllQuestions'))
    goToQuestionPage(questions.value.findIndex((_, index) => !questionAnswered(index)))
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
  if (isReviewDecision.value) selected.value[index] = []
  else {
    const option = customOption(index)
    if (!option) return
    selected.value[index] = question.multiple
      ? [...new Set([...(selected.value[index] || []), option.label])]
      : [option.label]
  }
  focusControlByKey(textareaControlKey(index))
}

async function confirmCurrentOption(target: Element | null) {
  if (!interaction.value?.pending || busy.value) return
  const index = currentQuestionIndex(target)
  if (index < 0) return
  const question = questions.value[index]
  if (!question) return

  if (isReviewDecision.value && controlKind(target) === 'option') {
    const label = target?.getAttribute('data-interaction-control-value') || ''
    if (label && !isOptionChecked(index, label)) toggleOption(index, label)
  }

  if (isReviewDecision.value) {
    await submitAnswers()
  } else {
    await confirmQuestion(index)
  }
}

async function confirmQuestion(index = questionPage.value) {
  if (!isQuestionFlow.value || busy.value || !questions.value[index]) return
  if (!questionAnswered(index)) {
    toasts.push('error', t('chat.attention.toasts.pleaseAnswerAllQuestions'))
    if (isCustomSelected(index)) openCustomEditor(index)
    return
  }
  if (index + 1 < questions.value.length) goToQuestionPage(index + 1)
  else await submitAnswers()
}

function moveArrowFocus(target: Element | null, direction: ActionCursorDirection) {
  const controls = interactionControls().filter((element) => controlKind(element) !== 'textarea')
  const current = target instanceof HTMLElement ? target : controls[0]
  if (!current) return
  let next = actionCursorNeighbour(
    transcriptActionRect(current),
    controls.filter((element) => element !== current).map((value) => ({ value, rect: transcriptActionRect(value) })),
    direction,
  )
  if (!next && (direction === 'left' || direction === 'right'))
    next = controls[controls.indexOf(current) + (direction === 'right' ? 1 : -1)] || null
  if (next) focusControlElement(next)
  else if (direction === 'up' || direction === 'left') returnToTranscript()
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
        else await confirmQuestion(index)
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
  const directions: Record<string, ActionCursorDirection> = {
    ArrowUp: 'up',
    k: 'up',
    ArrowDown: 'down',
    j: 'down',
    ArrowLeft: 'left',
    h: 'left',
    ArrowRight: 'right',
    l: 'right',
  }
  if (directions[key]) {
    consumeKey(event)
    if (isQuestionFlow.value && (directions[key] === 'left' || directions[key] === 'right'))
      moveQuestionPage(directions[key] === 'right' ? 1 : -1)
    else moveArrowFocus(control, directions[key]!)
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
      if (isReviewDecision.value) await confirmCurrentOption(control)
      else {
        const index = currentQuestionIndex(control)
        const label = control?.getAttribute('data-interaction-control-value') || ''
        if (index >= 0 && label) toggleOption(index, label)
      }
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
      if (index >= 0) {
        if (isReviewDecision.value) toggleCustom(index)
        else await confirmQuestion(index)
      }
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
    class="interaction-part min-w-0 py-1"
    data-transcript-interaction-part="true"
    :data-interaction-question-flow="isQuestionFlow || undefined"
    :data-interaction-request-id="requestId"
    :data-interaction-session-id="sessionId || ''"
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
      <div class="interaction-header">
        <div class="flex min-w-0 items-center gap-2 text-[13px] font-semibold">
          <RiShieldKeyholeLine class="h-4 w-4 text-primary" aria-hidden="true" />
          {{ t('chat.attention.ui.permissionLabel') }}
        </div>
        <span class="interaction-status" :data-pending="isPendingPermission || undefined" data-interaction-status>
          {{ permissionStatusLabel }}
        </span>
      </div>
      <div class="interaction-width">
        <p class="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">{{ permission.action }}</p>
        <p v-if="permission.reason" class="mt-2 text-[13px] leading-relaxed">{{ permission.reason }}</p>
        <p
          v-if="permission.explanation && permission.explanation !== permission.reason"
          class="mt-1 text-xs leading-relaxed text-muted-foreground"
        >
          {{ permission.explanation }}
        </p>
        <p v-if="permission.provenance" class="mt-1 text-[10px] text-muted-foreground/75">
          {{ permission.provenance }}
        </p>
        <div v-if="permission.replyReason" class="interaction-result mt-2" data-interaction-result>
          <span class="interaction-section-label">{{ t('chat.attention.ui.replyReasonLabel') }}</span>
          <p class="interaction-result-value">{{ permission.replyReason }}</p>
        </div>
        <div v-if="isPendingPermission" class="interaction-workspace" data-transcript-chrome="true">
          <div class="part-actionbar">
            <Button
              size="xs"
              variant="outline"
              class="part-action"
              :disabled="busy"
              data-interaction-control="true"
              data-interaction-control-kind="permission"
              data-interaction-control-key="permission:once"
              @focus="markControlFocus"
              @click="replyPermission('once')"
            >
              <RiLoader4Line
                v-if="busy && activeControlKey === 'permission:once'"
                class="animate-spin"
                aria-hidden="true"
              />
              <RiCheckLine v-else aria-hidden="true" />{{ t('chat.attention.ui.allowOnce') }}
            </Button>
            <Button
              size="xs"
              variant="outline"
              class="part-action"
              :disabled="busy"
              data-interaction-control="true"
              data-interaction-control-kind="permission"
              data-interaction-control-key="permission:always"
              @focus="markControlFocus"
              @click="replyPermission('always')"
            >
              <RiLoader4Line
                v-if="busy && activeControlKey === 'permission:always'"
                class="animate-spin"
                aria-hidden="true"
              />
              <RiShieldKeyholeLine v-else aria-hidden="true" />
              {{ t('chat.attention.ui.alwaysAllow') }}
            </Button>
            <Button
              size="xs"
              variant="outline"
              class="part-action"
              data-tone="destructive"
              :disabled="busy"
              data-interaction-control="true"
              data-interaction-control-kind="permission"
              data-interaction-control-key="permission:reject"
              @focus="markControlFocus"
              @click="replyPermission('reject')"
            >
              <RiLoader4Line
                v-if="busy && activeControlKey === 'permission:reject'"
                class="animate-spin"
                aria-hidden="true"
              />
              <RiCloseCircleLine v-else aria-hidden="true" />
              {{ t('chat.attention.ui.rejectPermission') }}
            </Button>
          </div>
        </div>
      </div>
    </template>

    <template v-else-if="interaction">
      <p
        v-if="props.interaction?.pending && !interaction.pending"
        class="mb-2 text-xs text-muted-foreground"
        role="status"
      >
        {{ unavailableStatus }}
      </p>
      <div class="interaction-header">
        <div v-if="interaction.title" class="min-w-0 text-[13px] font-semibold">
          {{ displayText(interaction.title) }}
        </div>
        <span v-if="isQuestionFlow" class="interaction-status" aria-live="polite" data-interaction-page>
          {{
            isSummaryPage
              ? t('chat.attention.ui.reviewAnswers')
              : t('chat.attention.ui.questionPager', { current: questionPage + 1, total: questions.length })
          }}
        </span>
        <span
          v-else
          class="interaction-status"
          :data-pending="interaction.pending || undefined"
          data-interaction-status
        >
          <RiChat3Line v-if="interaction.pending" aria-hidden="true" />
          <RiCheckLine v-else-if="interaction.reply && !replyCancelled" aria-hidden="true" />
          {{ interaction.pending ? t('chat.attention.ui.awaitingUserInput') : replyStatusLabel }}
        </span>
      </div>
      <div v-if="interaction.bodyMarkdown && (!isQuestionFlow || questionPage === 0)" class="mb-2 min-w-0">
        <MarkdownRenderer :content="interaction.bodyMarkdown" mode="markdown" :stream="false" />
      </div>

      <div v-if="!interaction.pending" class="interaction-results interaction-width" data-interaction-result>
        <section
          v-for="({ question, answers }, questionIndex) in committedAnswers"
          :key="question.questionId || questionIndex"
          class="interaction-result"
        >
          <div class="interaction-section-label">
            {{
              isReviewDecision
                ? t('chat.planReview.decision')
                : displayText(question.header) || t('chat.attention.ui.numberedQuestion', { count: questionIndex + 1 })
            }}
          </div>
          <p v-if="!isReviewDecision" class="interaction-result-question">{{ displayText(question.question) }}</p>
          <ul v-if="answers.length" class="interaction-result-answers">
            <li v-for="answer in answers" :key="answer.value" class="interaction-result-answer">
              <span class="interaction-result-marker" aria-hidden="true">
                <component
                  :is="
                    answer.custom
                      ? RiChat3Line
                      : isReviewDecision
                        ? reviewOptionAppearance(answer.value).icon
                        : RiCheckLine
                  "
                />
              </span>
              <div class="min-w-0">
                <p class="interaction-result-value">{{ displayText(answer.value) }}</p>
                <p v-if="answer.description" class="interaction-result-description">
                  {{ displayText(answer.description) }}
                </p>
              </div>
            </li>
          </ul>
          <p v-if="!answers.length" class="interaction-result-description">{{ replyStatusLabel }}</p>
        </section>
      </div>

      <div
        v-else-if="isReviewDecision"
        class="interaction-workspace interaction-width"
        role="group"
        :aria-label="displayText(questions[0]?.question || '')"
        data-interaction-options
      >
        <div class="interaction-section-label">{{ t('chat.planReview.decision') }}</div>
        <p v-if="questions[0]?.question && !hasBody" class="text-xs text-muted-foreground">
          {{ displayText(questions[0].question) }}
        </p>
        <button
          v-for="option in visibleOptions(0)"
          :key="option.label"
          type="button"
          class="interaction-choice"
          :title="[displayText(option.label), displayText(option.description)].filter(Boolean).join(' — ')"
          :aria-label="displayText(option.label)"
          :aria-description="option.description ? displayText(option.description) : undefined"
          :data-selected="isOptionChecked(0, option.label) || undefined"
          :data-tone="reviewOptionAppearance(option.label).tone"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="option"
          :data-interaction-control-key="optionControlKey(0, option.label)"
          :data-interaction-control-value="option.label"
          data-interaction-question-index="0"
          @focus="markControlFocus"
          @click="confirmCurrentOption($event.currentTarget as Element)"
        >
          <span class="interaction-choice-marker" aria-hidden="true">
            <component :is="reviewOptionAppearance(option.label).icon" />
          </span>
          <span class="interaction-choice-copy">
            <span class="interaction-choice-label">{{ displayText(option.label) }}</span>
            <span v-if="option.description" class="interaction-choice-description">{{
              displayText(option.description)
            }}</span>
          </span>
          <RiLoader4Line
            v-if="busy && isOptionChecked(0, option.label)"
            class="animate-spin text-primary"
            aria-hidden="true"
          />
          <RiArrowRightSLine v-else class="interaction-choice-arrow" aria-hidden="true" />
        </button>
        <button
          v-if="questions[0] && customAllowed(0)"
          type="button"
          class="interaction-choice interaction-choice-secondary"
          :aria-expanded="Boolean(customOpen[0])"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="custom"
          :data-interaction-control-key="customControlKey(0)"
          data-interaction-question-index="0"
          @focus="markControlFocus"
          @click="toggleCustom(0)"
        >
          <RiChat3Line aria-hidden="true" />
          <span class="interaction-choice-label">{{ t('chat.attention.ui.sendFeedback') }}</span>
          <RiArrowRightSLine :class="customOpen[0] ? 'rotate-90' : ''" aria-hidden="true" />
        </button>
        <textarea
          v-if="questions[0] && customAllowed(0) && customOpen[0]"
          v-model="customDrafts[0]"
          rows="3"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="textarea"
          :data-interaction-control-key="textareaControlKey(0)"
          data-interaction-question-index="0"
          @focus="markControlFocus"
          class="w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-xs leading-relaxed outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
          :aria-label="t('chat.attention.ui.customAnswerFor', { question: displayText(questions[0]?.question || '') })"
        />
      </div>

      <div v-else-if="isSummaryPage" class="interaction-workspace interaction-width" data-interaction-summary>
        <button
          v-for="(question, questionIndex) in questions"
          :key="question.questionId || questionIndex"
          type="button"
          class="interaction-choice"
          :disabled="busy"
          :title="`${displayText(question.header || question.question)}: ${answerState[questionIndex]?.map(displayText).join(', ') || t('chat.attention.ui.unanswered')}`"
          data-interaction-control="true"
          data-interaction-control-kind="summary"
          :data-interaction-control-key="`summary:${questionIndex}`"
          :aria-label="`${t('chat.attention.ui.editAnswer')}: ${displayText(question.header || question.question)}`"
          @focus="markControlFocus"
          @click="goToQuestionPage(questionIndex)"
        >
          <span class="interaction-choice-marker font-mono text-[11px]" aria-hidden="true">{{
            questionIndex + 1
          }}</span>
          <span class="interaction-choice-copy">
            <span class="interaction-choice-label">{{ displayText(question.header || question.question) }}</span>
            <span class="interaction-choice-description">{{
              answerState[questionIndex]?.map(displayText).join(', ') || t('chat.attention.ui.unanswered')
            }}</span>
          </span>
          <RiEditLine class="text-muted-foreground" aria-hidden="true" />
          <span class="shrink-0 text-[11px] text-muted-foreground">{{ t('chat.attention.ui.editAnswer') }}</span>
        </button>
      </div>

      <div v-else class="interaction-width space-y-2">
        <p v-if="!questions.length" class="text-xs text-muted-foreground">
          {{ t('chat.attention.ui.noQuestionsAvailable') }}
        </p>
        <section
          v-for="{ question, questionIndex } in visibleQuestions"
          :key="question.questionId || questionIndex"
          role="group"
          :aria-label="displayText(question.question)"
          :aria-describedby="isPendingInteraction ? hintId : undefined"
          class="interaction-workspace"
          data-interaction-options
        >
          <div class="mb-1.5 flex items-start justify-between gap-2">
            <div class="min-w-0">
              <div v-if="question.header" class="mb-1 text-[10px] font-medium text-muted-foreground">
                {{ displayText(question.header) }}
              </div>
              <div class="text-[13px] font-medium leading-snug">{{ displayText(question.question) }}</div>
            </div>
            <span class="shrink-0 text-[10px] text-muted-foreground">{{
              question.multiple ? t('chat.attention.ui.multiple') : t('chat.attention.ui.single')
            }}</span>
          </div>
          <template v-for="option in questionOptions(questionIndex)" :key="option.label">
            <label
              class="interaction-choice"
              :title="
                [
                  option.synthetic ? t('chat.attention.ui.typeYourOwnAnswer') : displayText(option.label),
                  displayText(option.description),
                ]
                  .filter(Boolean)
                  .join(' — ')
              "
              :aria-disabled="busy || undefined"
              :data-selected="isOptionChecked(questionIndex, option.label) || undefined"
            >
              <input
                :type="question.multiple ? 'checkbox' : 'radio'"
                :name="`${instanceId}-question-${requestId}-${questionIndex}`"
                :checked="isOptionChecked(questionIndex, option.label)"
                :aria-label="option.synthetic ? t('chat.attention.ui.typeYourOwnAnswer') : displayText(option.label)"
                :aria-description="option.description ? displayText(option.description) : undefined"
                :disabled="busy"
                data-interaction-control="true"
                :data-interaction-control-kind="option.custom ? 'custom' : 'option'"
                :data-interaction-control-key="
                  option.custom ? customControlKey(questionIndex) : optionControlKey(questionIndex, option.label)
                "
                :data-interaction-control-value="option.label"
                :data-interaction-question-index="questionIndex"
                @focus="markControlFocus"
                class="h-3.5 w-3.5 shrink-0 accent-primary"
                @click="
                  option.custom &&
                  !question.multiple &&
                  isCustomSelected(questionIndex) &&
                  openCustomEditor(questionIndex)
                "
                @change="toggleOption(questionIndex, option.label)"
              />
              <span class="interaction-choice-copy"
                ><span class="interaction-choice-label">{{
                  option.synthetic ? t('chat.attention.ui.typeYourOwnAnswer') : displayText(option.label)
                }}</span
                ><span v-if="option.description" class="interaction-choice-description">{{
                  displayText(option.description)
                }}</span></span
              >
            </label>
            <textarea
              v-if="option.custom && customOpen[questionIndex]"
              v-model="customDrafts[questionIndex]"
              rows="3"
              :disabled="busy"
              data-interaction-control="true"
              data-interaction-control-kind="textarea"
              :data-interaction-control-key="textareaControlKey(questionIndex)"
              :data-interaction-question-index="questionIndex"
              @focus="markControlFocus"
              class="w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-xs leading-relaxed outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
              :placeholder="t('chat.attention.ui.typeYourOwnAnswer')"
              :aria-label="t('chat.attention.ui.customAnswerFor', { question: displayText(question.question) })"
            />
          </template>
        </section>
      </div>

      <div
        v-if="isQuestionFlow"
        class="interaction-width mt-2 flex flex-wrap items-center justify-between gap-1.5 border-t border-border/60 pt-2"
        data-transcript-chrome="true"
      >
        <Button
          size="xs"
          variant="ghost"
          class="part-action"
          :disabled="busy || questionPage === 0"
          data-interaction-control="true"
          data-interaction-control-kind="navigation"
          data-interaction-control-key="page:previous"
          @focus="markControlFocus"
          @click="goToQuestionPage(questionPage - 1)"
          ><RiArrowLeftSLine aria-hidden="true" />{{ t('chat.attention.ui.previousQuestion') }}</Button
        >
        <Button
          v-if="!isSummaryPage && questionPage + 1 === questions.length"
          size="xs"
          variant="ghost"
          class="part-action"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="navigation"
          data-interaction-control-key="page:review"
          @focus="markControlFocus"
          @click="goToQuestionPage(questions.length)"
          >{{ t('chat.attention.ui.reviewAnswers') }}</Button
        >
        <Button
          v-if="!isSummaryPage"
          size="xs"
          variant="outline"
          class="part-action"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="navigation"
          data-interaction-control-key="page:next"
          @focus="markControlFocus"
          @click="confirmQuestion()"
          >{{
            questionPage + 1 === questions.length
              ? t('chat.attention.ui.sendAnswers')
              : t('chat.attention.ui.nextQuestion')
          }}
          <RiArrowRightSLine aria-hidden="true"
        /></Button>
      </div>
      <div
        v-if="interaction.pending && (isReviewDecision || isSummaryPage)"
        class="interaction-width mt-2 flex flex-wrap items-center gap-1.5"
        data-transcript-chrome="true"
      >
        <Button
          size="xs"
          variant="outline"
          class="part-action"
          :disabled="busy"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:reject"
          @focus="markControlFocus"
          @click="rejectAnswers"
          ><RiTimeLine aria-hidden="true" />{{ t('chat.attention.ui.dismissRequest') }}</Button
        >
        <Button
          v-if="!isReviewDecision || customOpen[0]"
          size="xs"
          class="part-action"
          data-variant="primary"
          :disabled="!canSubmit"
          data-interaction-control="true"
          data-interaction-control-kind="action"
          data-interaction-control-key="action:submit"
          @focus="markControlFocus"
          @click="submitAnswers"
          ><RiLoader4Line v-if="busy" class="animate-spin" aria-hidden="true" /><RiCheckLine
            v-else
            aria-hidden="true"
          />{{ t(isReviewDecision ? 'chat.attention.ui.sendFeedback' : 'chat.attention.ui.sendAnswers') }}</Button
        >
      </div>
    </template>
    <p
      v-if="isPendingInteraction"
      :id="hintId"
      class="interaction-width mt-2 text-[10px] leading-snug text-muted-foreground"
      data-transcript-chrome="true"
    >
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
