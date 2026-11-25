<script setup lang="ts">
import { onMounted, ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCheckLine, RiCloseLine, RiLoader4Line } from '@remixicon/vue'
import SidebarIconButton from '@/components/ui/SidebarIconButton.vue'
import { isPlainNavigationKey } from '@/lib/keyboardNavigation'

const props = defineProps<{
  label: string
  placeholder?: string
  submitLabel?: string
  busyLabel?: string
  cancelLabel?: string
  busy?: boolean
  invalid?: boolean
  error?: string
  compact?: boolean
  monospace?: boolean
}>()
const value = defineModel<string>({ required: true })
const emit = defineEmits<{ submit: []; cancel: [] }>()
const { t } = useI18n()
const input = ref<HTMLInputElement | null>(null)
const errorId = useId()

function focus(select = false) {
  input.value?.focus({ preventScroll: true })
  if (select) input.value?.select()
}

function onKeydown(event: KeyboardEvent) {
  if (!isPlainNavigationKey(event) || event.shiftKey || !['Enter', 'Escape'].includes(event.key)) return
  event.preventDefault()
  event.stopPropagation()
  if (props.busy) return
  if (event.key === 'Escape') emit('cancel')
  else emit('submit')
}

defineExpose({ input, focus })
onMounted(() => focus(true))
</script>

<template>
  <div class="grid w-full min-w-0 gap-1" data-inline-name-editor @click.stop>
    <div class="flex min-w-0 items-center gap-1">
      <input
        ref="input"
        v-model="value"
        data-keyboard-primary
        type="text"
        :aria-label="label"
        :placeholder="placeholder || label"
        :readonly="busy"
        :aria-invalid="error ? true : undefined"
        :aria-describedby="error ? errorId : undefined"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        class="w-full min-w-0 rounded-md border border-input bg-background px-1.5 text-xs text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
        :class="[compact ? 'h-5' : 'h-7', monospace && 'font-mono']"
        @keydown="onKeydown"
      />
      <SidebarIconButton
        size="sm"
        :tooltip="cancelLabel || t('common.cancel')"
        :disabled="busy"
        @click="emit('cancel')"
      >
        <RiCloseLine class="size-3.5" aria-hidden="true" />
      </SidebarIconButton>
      <SidebarIconButton
        size="sm"
        class="text-primary"
        :tooltip="busy ? busyLabel || t('common.saving') : submitLabel || t('common.save')"
        :disabled="busy || invalid || !value.trim()"
        @click="emit('submit')"
      >
        <RiLoader4Line v-if="busy" class="size-3.5 animate-spin" aria-hidden="true" />
        <RiCheckLine v-else class="size-3.5" aria-hidden="true" />
      </SidebarIconButton>
    </div>
    <p v-if="error" :id="errorId" class="text-xs text-destructive [overflow-wrap:anywhere]" role="alert">{{ error }}</p>
  </div>
</template>
