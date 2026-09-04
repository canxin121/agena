<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import { appTextPromptRequest, resolveAppTextPrompt } from '@/lib/appTextPrompt'

const { t } = useI18n()
const draft = ref('')

watch(
  () => appTextPromptRequest.value?.id,
  () => {
    draft.value = String(appTextPromptRequest.value?.initialValue || '')
  },
  { immediate: true },
)

function cancel() {
  resolveAppTextPrompt(null)
}

function submit() {
  const value = draft.value.trim()
  if (!value) return
  resolveAppTextPrompt(value)
}

function onOpenChange(open: boolean) {
  if (!open) cancel()
}
</script>

<template>
  <FormDialog
    v-if="appTextPromptRequest"
    :open="true"
    :title="appTextPromptRequest.title"
    :description="appTextPromptRequest.description"
    max-width="max-w-md"
    @update:open="onOpenChange"
  >
    <div class="grid gap-3">
      <Input
        v-model="draft"
        autofocus
        :placeholder="appTextPromptRequest.placeholder"
        :aria-label="appTextPromptRequest.title"
        @keydown.enter.prevent="submit"
      />
      <div class="flex flex-wrap justify-end gap-2">
        <Button variant="secondary" @click="cancel">
          {{ appTextPromptRequest.cancelText || t('common.cancel') }}
        </Button>
        <Button :disabled="!draft.trim()" @click="submit">
          {{ appTextPromptRequest.confirmText || t('common.confirm') }}
        </Button>
      </div>
    </div>
  </FormDialog>
</template>
