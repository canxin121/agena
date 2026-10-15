<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import Button from '@/components/ui/Button.vue'
import { isImeComposing } from '@/lib/imeKeyboard'

const props = defineProps<{ open: boolean; count: number; modelValue: string; error: string; submitting: boolean }>()
const emit = defineEmits<{
  (event: 'update:open', value: boolean): void
  (event: 'update:modelValue', value: string): void
  (event: 'submit'): void
}>()
const { t } = useI18n()
const body = ref<HTMLElement | null>(null)
watch(
  () => props.error,
  async (error) => {
    if (!error || !props.open) return
    await nextTick()
    body.value?.querySelector('input')?.focus()
  },
)
function onKeydown(event: KeyboardEvent) {
  if (
    event.key !== 'Enter' ||
    event.defaultPrevented ||
    isImeComposing(event) ||
    !(event.target instanceof HTMLInputElement)
  )
    return
  event.preventDefault()
  event.stopPropagation()
  if (!props.submitting && props.modelValue.trim()) emit('submit')
}
</script>

<template>
  <FormDialog
    :open="open"
    :title="String(t('files.explorer.selection.bulkMoveTitle'))"
    :description="String(t('files.explorer.selection.bulkMoveDescription', { count }))"
    @update:open="!submitting && emit('update:open', $event)"
  >
    <div ref="body" class="space-y-3" @keydown="onKeydown">
      <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
      <Input
        :model-value="modelValue"
        :aria-label="String(t('files.explorer.selection.bulkMovePlaceholder'))"
        :placeholder="String(t('files.explorer.selection.bulkMovePlaceholder'))"
        :aria-invalid="Boolean(error)"
        class="h-9 font-mono"
        @update:model-value="emit('update:modelValue', String($event))"
      />
      <div class="flex items-center justify-end gap-2">
        <Button variant="ghost" :disabled="submitting" @click="emit('update:open', false)">{{
          t('common.cancel')
        }}</Button>
        <Button :disabled="submitting || !modelValue.trim()" @click="emit('submit')">{{
          submitting ? t('common.working') : t('files.explorer.selection.bulkMove')
        }}</Button>
      </div>
    </div>
  </FormDialog>
</template>
