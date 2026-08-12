<script setup lang="ts">
import { computed } from 'vue'
import type { MessagePart, UserInputQuestion } from '@/agena/lib/agenaApi'
import { renderMarkdown } from '@/agena/lib/markdown'

/**
 * "Everything is a part": a pending interaction part (plan review or ask-user)
 * renders as a foldable inline form inside the message, auto-expanded on
 * arrival so the approval is immediately actionable. The part is the
 * interaction surface, not a separate card. After the reply lands the part
 * returns to its answered, non-interactive rendering.
 */

const props = defineProps<{
  part: MessagePart
  isInteractiveRequestBusy: (requestId: string) => boolean
  readUserAnswer: (requestId: string, questionIndex: string) => string
  updateUserAnswer: (requestId: string, questionIndex: string, value: string) => void
  submitUserAnswers: (requestId: string, sessionId?: number | null) => void | Promise<void>
  cancelUserAnswers: (requestId: string, sessionId?: number | null) => void | Promise<void>
}>()

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

/** The full typed request when present (`content.request`), else the flattened
 * `prompt`/`options` shape older payloads carry. */
const request = computed<Record<string, unknown>>(() => {
  const content = (props.part.content || {}) as Record<string, unknown>
  if (content.request && typeof content.request === 'object') {
    return content.request as Record<string, unknown>
  }
  return content
})

const title = computed(() => readString(request.value.title) || readString(request.value.prompt) || 'User input')
const bodyMarkdown = computed(() => readString(request.value.body_markdown) || '')
const kind = computed(() => readString(request.value.kind) || (request.value.type as string) || 'ask_user')

/** The request's session id, when the part carries it. The actions fall back
 * to the selected session when the pending resource lags behind the part. */
const sessionId = computed(() => {
  const value = request.value.session_id
  return typeof value === 'number' ? value : null
})

function submit() {
  void props.submitUserAnswers(requestId.value, sessionId.value)
}

function cancel() {
  void props.cancelUserAnswers(requestId.value, sessionId.value)
}

const questions = computed<UserInputQuestion[]>(() => {
  const list = Array.isArray(request.value.questions) ? (request.value.questions as UserInputQuestion[]) : []
  if (list.length) return list
  // Fallback: a single question synthesized from the flattened shape.
  const options = Array.isArray(request.value.options)
    ? request.value.options.map((option) =>
        typeof option === 'string' ? { label: option } : ((option as Record<string, unknown>) || {}),
      )
    : []
  const prompt = readString(request.value.prompt)
  if (!prompt && !options.length) return []
  return [
    {
      question: prompt || 'Choose an option',
      options: options as UserInputQuestion['options'],
      multiple: request.value.multiple === true,
      allow_custom: request.value.allow_custom === true,
    },
  ]
})

const requestId = computed(() => {
  const id = readString(request.value.request_id) || readString(request.value.id)
  if (!id) return ''
  return id
})

/** Question answers are keyed by positional index string (the backend sends no
 * question id), matching the TUI's `answers["0"]` reply shape. */
function questionIndex(index: number): string {
  return String(index)
}

function optionMarkup(label: string): string {
  return renderMarkdown(label)
}

function onInput(event: Event, index: number) {
  const value = (event.target as HTMLTextAreaElement | null)?.value || ''
  props.updateUserAnswer(requestId.value, questionIndex(index), value)
}
</script>

<template>
  <details class="message-input-activity" :open="true">
    <summary class="message-input-activity-head">
      <span class="badge">{{ kind === 'review' ? 'Plan review' : 'User input' }}</span>
      <strong>{{ title }}</strong>
    </summary>
    <div class="stack" style="margin-top: 10px">
      <div v-if="bodyMarkdown" class="markdown-body" v-html="renderMarkdown(bodyMarkdown)"></div>
      <div v-for="(question, index) in questions" :key="questionIndex(index)" class="field">
        <label class="label" :for="`${requestId}-${questionIndex(index)}`">
          {{ question.header || question.question }}
        </label>
        <div v-if="question.options && question.options.length" class="stack">
          <label
            v-for="option in question.options"
            :key="option.label"
            class="interaction-choice"
          >
            <input
              :type="question.multiple ? 'checkbox' : 'radio'"
              :name="`${requestId}-${questionIndex(index)}`"
              :value="option.label"
              :disabled="props.isInteractiveRequestBusy(requestId)"
              :checked="
                question.multiple
                  ? props
                      .readUserAnswer(requestId, questionIndex(index))
                      .split(',')
                      .map((value) => value.trim())
                      .includes(option.label)
                  : props.readUserAnswer(requestId, questionIndex(index)) === option.label
              "
              @change="
                props.updateUserAnswer(
                  requestId,
                  questionIndex(index),
                  question.multiple
                    ? [option.label, props.readUserAnswer(requestId, questionIndex(index))]
                        .filter(Boolean)
                        .filter((value, position, all) => all.indexOf(value) === position)
                        .join(',')
                    : option.label,
                )
              "
            />
            <span v-html="optionMarkup(option.label)"></span>
            <span v-if="option.description" class="muted">{{ option.description }}</span>
          </label>
        </div>
        <textarea
          :id="`${requestId}-${questionIndex(index)}`"
          class="textarea"
          :disabled="props.isInteractiveRequestBusy(requestId)"
          :value="props.readUserAnswer(requestId, questionIndex(index))"
          :placeholder="question.multiple ? 'comma,separated,values' : 'Your answer…'"
          @input="onInput($event, index)"
        />
      </div>
      <div class="button-row" style="margin-top: 12px">
        <button
          class="button primary"
          :disabled="props.isInteractiveRequestBusy(requestId)"
          @click="submit"
        >
          Submit
        </button>
        <button
          class="button danger"
          :disabled="props.isInteractiveRequestBusy(requestId)"
          @click="cancel"
        >
          Cancel
        </button>
      </div>
    </div>
  </details>
</template>
