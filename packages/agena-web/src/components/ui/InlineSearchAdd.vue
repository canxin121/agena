<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCheckLine } from '@remixicon/vue'

import Input from '@/components/ui/Input.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import { cn } from '@/lib/utils'
import { isPlainNavigationKey, isVisibleKeyboardTarget, navigationIndex } from '@/lib/keyboardNavigation'

const props = withDefaults(
  defineProps<{
    options: PickerOption[]
    modelValue?: string
    placeholder?: string
    panelTitle?: string
    allowCustom?: boolean
    monospace?: boolean
    maxInitialOptions?: number
    selectedValues?: string[]
    disabled?: boolean
  }>(),
  {
    modelValue: undefined,
    placeholder: 'Search…',
    panelTitle: '',
    allowCustom: true,
    monospace: false,
    maxInitialOptions: 80,
    selectedValues: () => [],
    disabled: false,
  },
)

const emit = defineEmits<{
  // Raw may contain separators; callers can reuse existing splitTags logic.
  (e: 'add', raw: string): void
  (e: 'remove', value: string): void
  (e: 'backspace-empty'): void
  (e: 'update:modelValue', value: string): void
}>()

const rootRef = ref<HTMLElement | null>(null)
const panelId = useId()
const { t } = useI18n()

const open = ref(false)
const internalQuery = ref('')

const query = computed<string>({
  get: () => (typeof props.modelValue === 'string' ? props.modelValue : internalQuery.value),
  set: (v) => {
    if (typeof props.modelValue === 'string') {
      emit('update:modelValue', v)
      return
    }
    internalQuery.value = v
  },
})

const q = computed(() => query.value.trim().toLowerCase())
const placeholderText = computed(() => {
  const raw = String(props.placeholder || '').trim()
  return !raw || raw === 'Search…' ? t('ui.inlineSearch.placeholder') : raw
})
const selectedSet = computed(
  () => new Set((props.selectedValues || []).map((v) => String(v || '').trim()).filter(Boolean)),
)

const filteredOptions = computed(() => {
  const list = Array.isArray(props.options) ? props.options : []
  const qq = q.value
  if (!qq) return list
  return list.filter((opt) => {
    const hay = `${opt.value} ${opt.label || ''} ${opt.description || ''}`.toLowerCase()
    return hay.includes(qq)
  })
})

const showOptions = computed(() => {
  if (q.value) return filteredOptions.value.slice(0, 200)
  return filteredOptions.value.slice(0, Math.max(0, props.maxInitialOptions))
})

const isTruncated = computed(() => !q.value && filteredOptions.value.length > showOptions.value.length)

const canOfferCustom = computed(() => {
  if (!props.allowCustom) return false
  const raw = query.value.trim()
  if (!raw) return false
  return true
})

function addRaw(raw: string) {
  const v = String(raw || '').trim()
  if (!v) return
  emit('add', v)
  query.value = ''
  // Keep the menu open for rapid entry.
  open.value = true
  void nextTick(() => rootRef.value?.querySelector('input')?.focus())
}

function addCustom() {
  addRaw(query.value)
}

function chooseOption(value: string) {
  const v = String(value || '').trim()
  if (!v) return
  if (selectedSet.value.has(v)) {
    emit('remove', v)
    query.value = ''
    open.value = true
    void nextTick(() => rootRef.value?.querySelector('input')?.focus())
    return
  }
  addRaw(v)
}

function onKeydown(e: KeyboardEvent) {
  if (!isPlainNavigationKey(e) || e.shiftKey) return
  const input = e.target instanceof HTMLInputElement
  if (input && ['ArrowDown', 'ArrowUp'].includes(e.key) && !open.value) {
    e.preventDefault()
    open.value = true
    void nextTick(() => onKeydown(new KeyboardEvent('keydown', { key: e.key })))
    return
  }
  if (open.value && (['ArrowDown', 'ArrowUp'].includes(e.key) || (!input && ['Home', 'End'].includes(e.key)))) {
    const buttons = Array.from(
      rootRef.value?.querySelectorAll<HTMLElement>('[data-inline-option]:not(:disabled)') || [],
    ).filter(isVisibleKeyboardTarget)
    const current = buttons.findIndex((button) => button === document.activeElement)
    const index = navigationIndex(e.key, current, buttons.length)
    if (index !== null) {
      e.preventDefault()
      e.stopPropagation()
      buttons[index]!.focus({ preventScroll: true })
      buttons[index]!.scrollIntoView({ block: 'nearest' })
    }
    return
  }
  if (e.key === 'Escape' && open.value) {
    e.preventDefault()
    e.stopPropagation()
    open.value = false
    rootRef.value?.querySelector('input')?.focus()
    // Focusing the input opens suggestions; Escape must leave them closed.
    open.value = false
    return
  }
  if (!input) return
  if (e.key === 'Enter') {
    e.preventDefault()
    if (canOfferCustom.value) {
      addCustom()
      return
    }
    const first = showOptions.value.find((option) => !option.disabled)
    if (first && !first.disabled) chooseOption(first.value)
    return
  }
  if (e.key === 'Backspace' && !query.value.trim()) {
    emit('backspace-empty')
  }
}

function onPaste(e: ClipboardEvent) {
  const text = e.clipboardData?.getData('text') || ''
  if (!text.trim()) return
  // If it looks like a tag list, add immediately (preserves existing paste UX).
  if (/[\n,\t ]/.test(text.trim())) {
    e.preventDefault()
    addRaw(text)
  }
}

function onFocus() {
  open.value = true
}

function closeIfOutside(ev: MouseEvent) {
  const root = rootRef.value
  const target = ev.target as Node | null
  if (!root || !target) return
  if (!root.contains(target)) open.value = false
}

onMounted(() => {
  document.addEventListener('mousedown', closeIfOutside)
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', closeIfOutside)
})

watch(
  () => props.disabled,
  (v) => {
    if (v) open.value = false
  },
)

function optionLabel(opt: PickerOption): string {
  return (opt.label || opt.value || '').trim()
}
</script>

<template>
  <div
    ref="rootRef"
    class="relative"
    @keydown="onKeydown"
    @focusout="
      (event) => {
        if (!rootRef?.contains(event.relatedTarget as Node | null)) open = false
      }
    "
  >
    <Input
      v-model="query"
      :placeholder="placeholderText"
      :class="cn('w-full', monospace ? 'font-mono' : '')"
      :disabled="disabled"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="open ? panelId : undefined"
      @focus="onFocus"
      @paste="onPaste"
    />

    <div
      v-if="open"
      :id="panelId"
      role="dialog"
      :aria-label="panelTitle || placeholderText"
      data-keyboard-overlay="true"
      class="absolute left-0 right-0 top-full mt-2 rounded-xl border border-border bg-background/95 text-foreground shadow-lg outline-none overflow-hidden z-20"
    >
      <div v-if="panelTitle" class="px-3 py-2 border-b border-border/40 text-xs font-semibold text-foreground">
        {{ panelTitle }}
      </div>

      <div class="max-h-64 overflow-auto p-1">
        <button
          v-if="canOfferCustom"
          data-inline-option
          type="button"
          class="w-full flex items-center justify-between gap-2 rounded-lg px-3 py-2 text-left hover:bg-secondary/40"
          @mousedown.prevent
          @click="addCustom"
        >
          <span class="text-xs truncate">{{ t('ui.inlineSearch.useValue', { value: query.trim() }) }}</span>
        </button>

        <div v-if="canOfferCustom" class="h-px bg-border/40 my-1" />

        <button
          v-for="opt in showOptions"
          data-inline-option
          :key="opt.value"
          type="button"
          class="w-full flex items-start justify-between gap-2 rounded-lg px-3 py-2 text-left hover:bg-secondary/40 disabled:opacity-50 disabled:pointer-events-none"
          :disabled="opt.disabled"
          @mousedown.prevent
          @click="chooseOption(opt.value)"
        >
          <span class="min-w-0">
            <span class="block text-xs truncate" :class="monospace ? 'font-mono' : ''">{{ optionLabel(opt) }}</span>
            <span v-if="opt.description" class="block text-[11px] text-muted-foreground truncate">{{
              opt.description
            }}</span>
          </span>
          <RiCheckLine v-if="selectedSet.has(opt.value)" class="h-4 w-4 text-primary flex-shrink-0" />
        </button>

        <div v-if="isTruncated" class="px-3 py-2 text-[11px] text-muted-foreground">
          {{ t('ui.inlineSearch.truncated', { shown: showOptions.length, total: filteredOptions.length }) }}
        </div>
        <div v-else-if="q && filteredOptions.length === 0" class="px-3 py-3 text-xs text-muted-foreground">
          {{ t('ui.inlineSearch.noMatches') }}
        </div>
      </div>
    </div>
  </div>
</template>
