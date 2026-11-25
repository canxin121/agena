<script setup lang="ts">
import { ref, watch } from 'vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import NameInputPrompt from '@/components/ui/NameInputPrompt.vue'
import { appDialogRequest, finishAppDialog } from '@/lib/appDialogQueue'

const request = appDialogRequest
const draft = ref('')
watch(
  request,
  (active) => {
    draft.value = active?.kind === 'text' ? (active.options.initialValue ?? '') : ''
  },
  { immediate: true },
)

function submitText(id: number) {
  const value = draft.value.trim()
  if (value) finishAppDialog(id, value)
}
</script>

<template>
  <ConfirmPopover
    v-if="request?.kind === 'confirm'"
    :key="request.id"
    :open="true"
    force-dialog
    :close-on-confirm="false"
    :close-on-cancel="false"
    v-bind="request.options"
    @confirm="finishAppDialog(request.id, true)"
    @cancel="finishAppDialog(request.id, false)"
    @update:open="!$event && finishAppDialog(request.id, false)"
  />
  <NameInputPrompt
    v-else-if="request?.kind === 'text'"
    :key="request.id"
    v-model="draft"
    open
    dialog
    :title="request.options.title"
    :description="request.options.description"
    :placeholder="request.options.placeholder"
    :cancel-label="request.options.cancelText"
    :confirm-label="request.options.confirmText"
    @cancel="finishAppDialog(request.id, null)"
    @submit="submitText(request.id)"
  />
</template>
