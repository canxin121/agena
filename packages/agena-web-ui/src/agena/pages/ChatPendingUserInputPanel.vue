<script setup lang="ts">
import type { UserInputRequest } from '@/agena/lib/agenaApi'

const props = defineProps<{
  requests: UserInputRequest[]
  isInteractiveRequestBusy: (requestId: string) => boolean
  readUserAnswer: (requestId: string, questionId: string) => string
  updateUserAnswer: (requestId: string, questionId: string, value: string) => void
  submitUserAnswers: (requestId: string) => void | Promise<void>
  cancelUserAnswers: (requestId: string) => void | Promise<void>
}>()
</script>

<template>
  <section v-if="props.requests.length" class="card">
    <h3>Pending User Input</h3>
    <div class="list">
      <div v-for="request in props.requests" :key="request.request_id" class="list-item">
        <div>
          <strong>{{ request.request_id }}</strong>
        </div>
        <div class="stack" style="margin-top: 10px">
          <div v-for="question in request.questions" :key="question.id" class="field">
            <label class="label" :for="`${request.request_id}-${question.id}`">
              {{ question.header || question.question }}
            </label>
            <textarea
              :id="`${request.request_id}-${question.id}`"
              class="textarea"
              :disabled="props.isInteractiveRequestBusy(request.request_id)"
              :value="props.readUserAnswer(request.request_id, question.id)"
              :placeholder="question.multiple ? 'comma,separated,values' : question.question"
              @input="
                props.updateUserAnswer(
                  request.request_id,
                  question.id,
                  ($event.target as HTMLTextAreaElement | null)?.value || '',
                )
              "
            />
          </div>
        </div>
        <div class="button-row" style="margin-top: 12px">
          <button
            class="button primary"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.submitUserAnswers(request.request_id)"
          >
            Submit Answers
          </button>
          <button
            class="button danger"
            :disabled="props.isInteractiveRequestBusy(request.request_id)"
            @click="props.cancelUserAnswers(request.request_id)"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  </section>
</template>
