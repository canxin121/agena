<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Input from '@/components/ui/Input.vue'
import { isPlainNavigationKey, navigationIndex } from '@/lib/keyboardNavigation'

const props = defineProps<{ options: readonly string[]; loading: boolean }>()
const branch = defineModel<string>({ required: true })
const { t } = useI18n()
const fieldId = useId()
const listId = `${fieldId}-branches`
const open = ref(false)
const active = ref(-1)
const options = computed(() => props.options.slice(0, 20))
const expanded = computed(() => open.value && !props.loading && options.value.length > 0)

watch(options, () => {
  active.value = -1
})

function close() {
  open.value = false
  active.value = -1
}

function choose(value: string) {
  branch.value = value
  close()
}

function onKeydown(event: KeyboardEvent) {
  if (!isPlainNavigationKey(event) || event.shiftKey) return

  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    open.value = true
    if (!expanded.value) return
    const index = navigationIndex(event.key, active.value, options.value.length)
    if (index === null) return
    event.preventDefault()
    active.value = index
    void nextTick(() => document.getElementById(optionId(index))?.scrollIntoView({ block: 'nearest' }))
  } else if (expanded.value && event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    close()
  } else if (expanded.value && event.key === 'Enter') {
    const value = options.value[active.value]
    if (value === undefined) return
    event.preventDefault()
    choose(value)
  }
}

function optionId(index: number) {
  return `${listId}-${index}`
}
</script>

<template>
  <label :for="fieldId" class="text-xs font-medium text-muted-foreground">{{ t('git.fields.branch') }}</label>
  <Input
    :id="fieldId"
    v-model="branch"
    role="combobox"
    aria-autocomplete="list"
    :aria-expanded="expanded"
    :aria-controls="expanded ? listId : undefined"
    :aria-activedescendant="expanded && active >= 0 ? optionId(active) : undefined"
    class="h-9 font-mono text-xs"
    placeholder="main"
    @focus="open = true"
    @blur="close"
    @input="open = true"
    @keydown="onKeydown"
  />
  <p v-if="loading" role="status" class="text-xs text-muted-foreground">
    {{ t('git.remoteTargetDialogs.loadingRemoteBranches') }}
  </p>
  <ul
    v-if="expanded"
    :id="listId"
    role="listbox"
    :aria-label="t('git.fields.branch')"
    class="max-h-36 overflow-auto overscroll-contain rounded-md border border-border/60 p-1"
  >
    <li
      v-for="(value, index) in options"
      :id="optionId(index)"
      :key="value"
      role="option"
      :aria-selected="index === active"
      class="cursor-pointer rounded px-2 py-1.5 font-mono text-xs [overflow-wrap:anywhere]"
      :class="index === active ? 'bg-secondary text-secondary-foreground' : ''"
      @pointerdown.prevent
      @mousemove="active = index"
      @click="choose(value)"
    >
      {{ value }}
    </li>
  </ul>
</template>
