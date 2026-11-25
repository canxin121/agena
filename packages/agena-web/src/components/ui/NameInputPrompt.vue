<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import { isImeComposing } from '@/lib/imeKeyboard'
import { isPlainNavigationKey, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    open: boolean
    modelValue: string
    title?: string
    description?: string
    placeholder?: string
    confirmLabel?: string
    cancelLabel?: string
    busy?: boolean
    isCompactTouch?: boolean
    dialog?: boolean
  }>(),
  { isCompactTouch: undefined },
)
const emit = defineEmits<{
  'update:open': [value: boolean]
  'update:modelValue': [value: string]
  submit: []
  cancel: []
}>()

const ui = useUiStore()
const { t } = useI18n()
const form = ref<HTMLFormElement | null>(null)
const compact = computed(() => props.isCompactTouch ?? ui.isCompactTouch)
const modal = computed(() => props.dialog || compact.value)
const containerProps = computed(() =>
  modal.value
    ? {
        open: props.open,
        title: props.title,
        description: props.description,
        maxWidth: 'max-w-md',
        isCompactTouch: compact.value,
        initialFocus: '[data-name-input]',
      }
    : { class: 'rounded-lg border border-border/70 bg-background/95 p-2' },
)

const draft = computed({
  get: () => props.modelValue,
  set: (value: string | number) => emit('update:modelValue', String(value)),
})
const canSubmit = computed(() => !props.busy && Boolean(draft.value.trim()))

function close() {
  emit('cancel')
  emit('update:open', false)
}

function submit() {
  if (canSubmit.value) emit('submit')
}

function onKeydown(event: KeyboardEvent) {
  const plain = isPlainNavigationKey(event) && !event.shiftKey
  if (!modal.value && event.key === 'Escape' && plain) {
    event.preventDefault()
    event.stopPropagation()
    close()
  }
  if (event.key === 'Enter' && event.target instanceof HTMLInputElement && !isImeComposing(event)) {
    event.preventDefault()
    if (plain) submit()
  }
}

watch(
  form,
  (element) => {
    const input = element?.querySelector('input')
    if (!input) return
    input.focus({ preventScroll: true })
    input.select()
  },
  { flush: 'post' },
)

let inlineTrigger: HTMLElement | null = null
watch(
  () => props.open,
  (open) => {
    if (typeof document === 'undefined') return
    if (open) {
      const active = document.activeElement
      inlineTrigger = !modal.value && active instanceof HTMLElement ? active : null
      return
    }
    const trigger = inlineTrigger
    inlineTrigger = null
    if (!trigger) return
    void nextTick(() => {
      if (!props.open && document.activeElement === document.body && isVisibleKeyboardTarget(trigger)) {
        trigger.focus({ preventScroll: true })
      }
    })
  },
  { immediate: true, flush: 'sync' },
)
</script>

<template>
  <component :is="modal ? FormDialog : 'div'" v-if="open" v-bind="containerProps" @update:open="!$event && close()">
    <form
      ref="form"
      class="flex gap-2"
      :class="modal ? 'flex-col gap-4' : 'items-center'"
      @submit.prevent="submit"
      @keydown="onKeydown"
    >
      <Input
        v-model="draft"
        data-name-input
        class="min-w-0"
        :class="modal ? 'h-10 text-sm' : 'h-8 flex-1 text-xs'"
        :aria-label="title || placeholder"
        :placeholder="placeholder"
      />
      <div class="flex shrink-0 flex-wrap items-center justify-end gap-2">
        <Button type="button" variant="ghost" :size="modal ? 'sm' : 'xs'" :disabled="busy" @click="close">
          {{ cancelLabel || t('common.cancel') }}
        </Button>
        <Button type="submit" :size="modal ? 'sm' : 'xs'" :disabled="!canSubmit">
          {{ confirmLabel || t('common.confirm') }}
        </Button>
      </div>
    </form>
  </component>
</template>
