<script setup lang="ts">
import { computed, ref, watch, type Component } from 'vue'
import { RiArrowDownSLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import OptionMenu from '@/components/ui/OptionMenu.vue'
import PickerTrigger from '@/components/ui/PickerTrigger.vue'
import { normalizeOptionSearch, optionSearchText } from '@/components/ui/optionSearch'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    modelValue: string
    options: PickerOption[]
    title?: string
    emptyLabel?: string
    includeEmpty?: boolean
    emptyDisabled?: boolean
    searchPlaceholder?: string
    placeholder?: string
    allowCustom?: boolean
    monospace?: boolean
    disabled?: boolean
    size?: 'default' | 'sm'
    triggerClass?: string
    displayLabel?: string
    icon?: Component
    maxInitialOptions?: number
    desktopFixed?: boolean
    isCompactTouch?: boolean
    returnFocusTarget?: () => HTMLElement | null
  }>(),
  {
    emptyLabel: 'Select...',
    includeEmpty: true,
    emptyDisabled: false,
    searchPlaceholder: 'Search...',
    placeholder: 'Select...',
    allowCustom: false,
    monospace: false,
    disabled: false,
    size: 'default',
    triggerClass: '',
    maxInitialOptions: 60,
    desktopFixed: true,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
}>()

const { t } = useI18n()
const ui = useUiStore()

const trigger = ref<InstanceType<typeof PickerTrigger> | null>(null)
const open = ref(false)
const openAt = ref<'first' | 'last'>()
const query = ref('')
const effectiveIsMobilePointer = computed(() => props.isCompactTouch ?? ui.isCompactTouch)

const selected = computed(() => String(props.modelValue || '').trim())
const q = computed(() => normalizeOptionSearch(query.value))

const effectiveSearchPlaceholder = computed(() => {
  const raw = String(props.searchPlaceholder || '').trim()
  if (!raw || raw === 'Search...') return t('common.optionPicker.searchPlaceholder')
  return raw
})
const triggerAriaLabel = computed(() => {
  const pickerTitle = String(props.title || '').trim()
  const value = String(selectedDisplayLabel.value || '').trim()
  return pickerTitle && value && pickerTitle !== value ? `${pickerTitle}: ${value}` : pickerTitle || value
})

const selectedDisplayLabel = computed(() => {
  if (props.displayLabel !== undefined) return props.displayLabel
  if (!selected.value) {
    const placeholder = String(props.placeholder || '').trim()
    const emptyLabel = String(props.emptyLabel || '').trim()
    const effectivePlaceholder =
      !placeholder || placeholder === 'Select...' ? t('common.optionPicker.selectPlaceholder') : placeholder
    const effectiveEmptyLabel = emptyLabel || effectivePlaceholder
    if (!props.includeEmpty) return effectivePlaceholder
    return effectiveEmptyLabel || effectivePlaceholder
  }

  const opt = (Array.isArray(props.options) ? props.options : []).find((o) => o.value === selected.value)
  if (opt) return optionLabel(opt)
  return selected.value
})

const filteredOptions = computed(() => {
  const qq = q.value
  if (!qq) return props.options
  return props.options.filter((opt) => optionKeywords(opt).includes(qq))
})

const showOptions = computed(() => {
  if (q.value) return filteredOptions.value
  return filteredOptions.value.slice(0, Math.max(0, props.maxInitialOptions))
})

const isTruncated = computed(() => !q.value && filteredOptions.value.length > showOptions.value.length)

const canOfferCustom = computed(() => {
  if (!props.allowCustom) return false
  const raw = query.value.trim()
  if (!raw) return false
  return !props.options.some((o) => o.value === raw)
})

const helperText = computed(() => {
  if (isTruncated.value) {
    return t('common.optionPicker.truncatedHelper', {
      shown: showOptions.value.length,
      total: filteredOptions.value.length,
    })
  }
  return ''
})

const emptyText = computed(() => {
  if (canOfferCustom.value) return t('common.optionPicker.emptyNoPresetMatches')
  return t('common.optionPicker.emptyNoMatches')
})

const menuGroups = computed<OptionMenuGroup[]>(() => {
  const groups: OptionMenuGroup[] = []

  if (props.includeEmpty) {
    groups.push({
      id: 'default',
      items: [
        {
          id: '__empty__',
          label: props.emptyLabel,
          checked: !selected.value,
          disabled: props.emptyDisabled,
          keywords: 'auto default',
          description: '',
        },
      ],
    })
  }

  if (canOfferCustom.value) {
    groups.push({
      id: 'custom',
      title: t('common.optionPicker.customGroupTitle'),
      items: [
        {
          id: '__custom__',
          label: t('common.optionPicker.useValue', { value: query.value.trim() }),
          description: t('common.optionPicker.setCustomValue'),
          keywords: query.value,
          monospace: props.monospace,
        },
      ],
    })
  }

  const byGroup = new Map<string, OptionMenuGroup>()
  for (const opt of showOptions.value) {
    const title = opt.group || ''
    let group = byGroup.get(title)
    if (!group) {
      group = { id: title ? `options:${title}` : 'options', title, items: [] }
      byGroup.set(title, group)
    }
    group.items.push({
      id: `opt:${opt.value}`,
      label: optionLabel(opt),
      description: opt.description,
      checked: opt.value === selected.value,
      disabled: opt.disabled,
      keywords: optionKeywords(opt),
      monospace: props.monospace,
    })
  }
  groups.push(...byGroup.values())

  return groups
})

function optionKeywords(opt: PickerOption): string {
  return optionSearchText(opt.value, opt.label, opt.description, opt.group, opt.keywords)
}

watch(open, (isOpen) => {
  if (!isOpen) {
    query.value = ''
    openAt.value = undefined
  }
})

watch(
  () => props.disabled,
  (disabled) => {
    if (disabled) open.value = false
  },
)

function choose(value: string) {
  emit('update:modelValue', value)
  open.value = false
}

function chooseCustom() {
  const raw = query.value.trim()
  if (!raw) return
  choose(raw)
}

function handleSelect(item: OptionMenuItem) {
  if (item.id === '__empty__') {
    choose('')
    return
  }
  if (item.id === '__custom__') {
    chooseCustom()
    return
  }
  if (item.id.startsWith('opt:')) {
    choose(item.id.slice(4))
  }
}

function setOpen(value: boolean) {
  if (value && props.disabled) return
  open.value = value
}

function openFrom(edge: 'first' | 'last') {
  openAt.value = edge
  setOpen(true)
}

function setQuery(value: string) {
  query.value = String(value || '')
}

function optionLabel(opt: PickerOption): string {
  return (opt.label || opt.value || '').trim()
}
</script>

<template>
  <div class="relative min-w-0">
    <PickerTrigger
      ref="trigger"
      block
      :open="open"
      :disabled="disabled"
      :size="size"
      :class="triggerClass"
      :label="selectedDisplayLabel"
      :aria-label="triggerAriaLabel"
      :icon="icon"
      :monospace="monospace"
      @toggle="setOpen(!open)"
      @open-at="openFrom"
    >
      <RiArrowDownSLine class="h-4 w-4 shrink-0 text-muted-foreground/70" aria-hidden="true" />
    </PickerTrigger>

    <OptionMenu
      :open="open"
      :open-at="openAt"
      :query="query"
      :groups="menuGroups"
      :title="title"
      :searchable="true"
      :search-placeholder="effectiveSearchPlaceholder"
      :empty-text="emptyText"
      :helper-text="helperText"
      :is-compact-touch="effectiveIsMobilePointer"
      :desktop-fixed="desktopFixed"
      :desktop-anchor-el="trigger?.triggerEl"
      :return-focus-target="returnFocusTarget"
      desktop-placement="bottom-start"
      desktop-class="w-[min(420px,calc(100vw-2rem))]"
      filter-mode="external"
      @update:open="setOpen"
      @update:query="setQuery"
      @select="handleSelect"
    />
  </div>
</template>
