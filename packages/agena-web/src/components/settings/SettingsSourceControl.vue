<script setup lang="ts">
import { computed } from 'vue'
import { RiResetLeftLine } from '@remixicon/vue'
import IconButton from '@/components/ui/IconButton.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import SegmentedButton from '@/components/ui/SegmentedButton.vue'
import { settingsText as st } from '@/i18n/settingsText'

const props = withDefaults(
  defineProps<{
    modelValue: 'inherit' | 'local' | 'effective'
    label: string
    scope?: string
    inheritedValue?: string
    inheritedSource?: string
    showInheritedSummary?: boolean
    showInheritedValue?: boolean
    disabled?: boolean
    inheritDisabled?: boolean
    localDisabled?: boolean
  }>(),
  {
    scope: 'workspace',
    inheritedValue: '',
    inheritedSource: '',
    showInheritedSummary: true,
    showInheritedValue: true,
    disabled: false,
    inheritDisabled: false,
    localDisabled: false,
  },
)
const emit = defineEmits<{ 'update:modelValue': [value: 'inherit' | 'local'] }>()
const resetLabel = computed(() => (props.scope === 'global' ? st('Reset to defaults') : st('Use inherited value')))
const localLabel = computed(() => (props.scope === 'profile' ? st('This profile') : st('This scope')))
const localAction = computed(() => (props.scope === 'profile' ? st('Set in this profile') : st('Set in this layer')))
const inheritTitle = computed(() =>
  [resetLabel.value, props.scope === 'global' ? '' : props.inheritedSource, props.inheritedValue]
    .filter(Boolean)
    .join(' · '),
)
const inheritedSummary = computed(() =>
  [props.inheritedSource || st('Parent configuration'), props.showInheritedValue ? props.inheritedValue : '']
    .filter(Boolean)
    .join(' · '),
)
function select(value: 'inherit' | 'local') {
  if (
    props.disabled ||
    props.modelValue === 'effective' ||
    (props.scope === 'global' && value === 'local') ||
    props.modelValue === value ||
    (value === 'inherit' ? props.inheritDisabled : props.localDisabled)
  )
    return
  emit('update:modelValue', value)
}
</script>

<template>
  <span
    v-if="modelValue === 'effective'"
    class="setting-source-effective text-[13px] text-muted-foreground"
    data-setting-source="effective"
    >{{ $st('Effective') }}</span
  >
  <IconButton
    v-else-if="scope === 'global'"
    variant="ghost"
    size="sm"
    class="setting-reset-control order-last shrink-0"
    data-setting-reset
    data-setting-scope="global"
    :aria-label="label + ': ' + resetLabel"
    :tooltip="inheritTitle"
    :disabled="disabled || inheritDisabled || modelValue !== 'local'"
    @click="select('inherit')"
    ><RiResetLeftLine class="h-3.5 w-3.5" aria-hidden="true"
  /></IconButton>
  <SegmentedControl
    v-else
    role="group"
    class="setting-source-control inline-flex min-w-0 max-w-full shrink gap-0 p-0"
    :class="{ 'setting-source-control--compact': !showInheritedSummary }"
    :data-setting-source="modelValue"
    :data-setting-scope="scope"
    :aria-label="label + ' · ' + $st('Source')"
  >
    <SegmentedButton
      role="button"
      :aria-selected="undefined"
      :aria-pressed="modelValue === 'inherit'"
      :active="modelValue === 'inherit'"
      :disabled="disabled || inheritDisabled"
      :aria-label="label + ': ' + resetLabel"
      :title="inheritTitle"
      class="setting-source-choice min-w-0 flex-1"
      data-setting-source-choice="inherit"
      @click="select('inherit')"
      ><span class="shrink-0">{{ $st('Inherit') }}</span>
      <span
        v-if="modelValue === 'inherit' && showInheritedSummary"
        class="setting-source-inherited min-w-0 truncate text-[11px] font-normal text-muted-foreground"
        data-setting-inherited
        :title="inheritedSummary"
        ><span data-setting-inherited-source>{{ inheritedSource || $st('Parent configuration') }}</span>
        <template v-if="showInheritedValue && inheritedValue">
          · <span data-setting-inherited-value>{{ inheritedValue }}</span></template
        ></span
      ></SegmentedButton
    >
    <SegmentedButton
      role="button"
      :aria-selected="undefined"
      :aria-pressed="modelValue === 'local'"
      :active="modelValue === 'local'"
      :disabled="disabled || localDisabled"
      :aria-label="label + ': ' + localAction"
      :title="localAction"
      class="setting-source-choice shrink-0"
      data-setting-source-choice="local"
      @click="select('local')"
      >{{ localLabel }}</SegmentedButton
    >
  </SegmentedControl>
</template>

<style scoped>
.setting-source-control :deep(.setting-source-choice) {
  min-height: 32px;
  height: auto;
  padding: 0.25rem;
  font-size: 13px;
  line-height: 1.25;
  white-space: nowrap;
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
}
.setting-source-inherited {
  max-width: 10rem;
}
.setting-source-control--compact :deep(.setting-source-choice) {
  padding-inline: 0.4375rem;
}
.setting-source-control :deep(.setting-source-choice:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}
.setting-source-control :deep(.setting-source-choice[aria-pressed='true']) {
  color: var(--color-primary);
  background: color-mix(in oklch, var(--color-primary) 12%, transparent);
}
.setting-source-control :deep(.setting-source-choice:focus-visible) {
  outline: 2px solid var(--color-ring);
  outline-offset: 1px;
}
</style>
