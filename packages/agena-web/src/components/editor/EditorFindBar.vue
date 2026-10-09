<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowDownSLine, RiArrowRightSLine, RiArrowUpSLine, RiCloseLine, RiListCheck3 } from '@remixicon/vue'

import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import SegmentedButton from '@/components/ui/SegmentedButton.vue'

const props = withDefaults(
  defineProps<{
    modelValue: string
    replaceValue?: string
    currentMatch: number
    matchCount: number
    invalidRegex?: boolean
    caseSensitive: boolean
    wholeWord: boolean
    regex: boolean
    replaceVisible?: boolean
    replaceAllowed?: boolean
    replaceCurrentDisabled?: boolean
    replaceAllDisabled?: boolean
  }>(),
  {
    replaceValue: '',
    invalidRegex: false,
    replaceVisible: false,
    replaceAllowed: false,
    replaceCurrentDisabled: false,
    replaceAllDisabled: false,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'update:replaceValue', value: string): void
  (e: 'next'): void
  (e: 'previous'): void
  (e: 'toggle-replace'): void
  (e: 'replace'): void
  (e: 'replace-all'): void
  (e: 'toggle-case-sensitive'): void
  (e: 'toggle-whole-word'): void
  (e: 'toggle-regex'): void
  (e: 'close'): void
}>()

const rootRef = ref<HTMLElement | null>(null)
const { t } = useI18n()

function focusInput(selectAll = false) {
  nextTick(() => {
    const input = rootRef.value?.querySelector('input[data-oc-find-input="query"]')
    if (!(input instanceof HTMLInputElement)) return
    input.focus()
    if (selectAll) {
      input.select()
    }
  })
}

function focusReplace(selectAll = false) {
  nextTick(() => {
    const input = rootRef.value?.querySelector('input[data-oc-find-input="replace"]')
    if (!(input instanceof HTMLInputElement)) return
    input.focus()
    if (selectAll) {
      input.select()
    }
  })
}

function onInputKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    if (event.shiftKey) {
      emit('previous')
      return
    }
    emit('next')
    return
  }

  if (event.key === 'Escape') {
    event.preventDefault()
    emit('close')
  }
}

function onReplaceKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    if (event.metaKey || event.ctrlKey) {
      emit('replace-all')
      return
    }
    emit('replace')
    return
  }

  if (event.key === 'Escape') {
    event.preventDefault()
    emit('close')
  }
}

defineExpose({
  focusInput,
  focusReplace,
})
</script>

<template>
  <div
    ref="rootRef"
    class="oc-editor-find-bar absolute right-3 top-3 z-30 flex w-[min(40rem,calc(100%-1.5rem))] flex-wrap items-center gap-2 rounded-lg border border-border/80 bg-card/95 p-2 text-xs shadow-lg backdrop-blur"
  >
    <IconButton
      v-if="replaceAllowed"
      size="xs"
      variant="ghost"
      class="h-8 w-8 text-muted-foreground hover:text-foreground"
      :aria-label="replaceVisible ? t('editor.find.hideReplace') : t('editor.find.showReplace')"
      :title="replaceVisible ? t('editor.find.hideReplace') : t('editor.find.showReplace')"
      :tooltip="replaceVisible ? t('editor.find.hideReplace') : t('editor.find.showReplace')"
      @click="emit('toggle-replace')"
    >
      <component :is="replaceVisible ? RiArrowDownSLine : RiArrowRightSLine" class="h-4 w-4" />
    </IconButton>

    <div class="min-w-[12rem] flex-1">
      <Input
        :model-value="modelValue"
        type="text"
        :placeholder="t('editor.find.placeholderFind')"
        data-oc-find-input="query"
        class="h-8 px-2.5 font-mono text-xs"
        :aria-label="t('editor.find.placeholderFind')"
        @update:model-value="(value) => emit('update:modelValue', String(value ?? ''))"
        @keydown="onInputKeydown"
      />
    </div>

    <div class="min-w-[4.5rem] text-right font-mono text-[11px] text-muted-foreground">
      <span v-if="invalidRegex" class="text-destructive">{{ t('editor.find.invalid') }}</span>
      <span v-else-if="matchCount > 0">{{ currentMatch }} / {{ matchCount }}</span>
      <span v-else>0 / 0</span>
    </div>

    <div class="flex items-center gap-1 rounded-md bg-muted/55 px-1 py-1">
      <IconButton
        size="xs"
        variant="ghost"
        class="h-6 w-6 text-muted-foreground hover:text-foreground"
        :disabled="matchCount < 1 || invalidRegex"
        :title="t('editor.find.previousMatch')"
        :tooltip="t('editor.find.previousMatch')"
        :aria-label="t('editor.find.previousMatch')"
        @click="emit('previous')"
      >
        <RiArrowUpSLine class="h-4 w-4" />
      </IconButton>
      <IconButton
        size="xs"
        variant="ghost"
        class="h-6 w-6 text-muted-foreground hover:text-foreground"
        :disabled="matchCount < 1 || invalidRegex"
        :title="t('editor.find.nextMatch')"
        :tooltip="t('editor.find.nextMatch')"
        :aria-label="t('editor.find.nextMatch')"
        @click="emit('next')"
      >
        <RiArrowDownSLine class="h-4 w-4" />
      </IconButton>
    </div>

    <div class="flex items-center gap-1 rounded-md bg-muted/55 p-1">
      <SegmentedButton
        :active="caseSensitive"
        size="xs"
        class="h-6 px-1.5 font-mono text-[11px]"
        :aria-label="t('editor.find.matchCase')"
        :title="t('editor.find.matchCase')"
        @click="emit('toggle-case-sensitive')"
      >
        Aa
      </SegmentedButton>
      <SegmentedButton
        :active="wholeWord"
        size="xs"
        class="h-6 px-1.5 font-mono text-[11px]"
        :aria-label="t('editor.find.wholeWord')"
        :title="t('editor.find.wholeWord')"
        @click="emit('toggle-whole-word')"
      >
        W
      </SegmentedButton>
      <SegmentedButton
        :active="regex"
        size="xs"
        class="h-6 px-1.5 font-mono text-[11px]"
        :aria-label="t('editor.find.useRegex')"
        :title="t('editor.find.useRegex')"
        @click="emit('toggle-regex')"
      >
        .*
      </SegmentedButton>
    </div>

    <IconButton
      size="xs"
      variant="ghost"
      class="h-6 w-6 text-muted-foreground hover:bg-muted hover:text-foreground"
      :title="t('editor.find.close')"
      :tooltip="t('editor.find.close')"
      :aria-label="t('editor.find.close')"
      @click="emit('close')"
    >
      <RiCloseLine class="h-4 w-4" />
    </IconButton>

    <div v-if="replaceAllowed && replaceVisible" class="flex w-full items-center gap-2 border-t border-border/70 pt-2">
      <div class="min-w-[12rem] flex-1">
        <Input
          :model-value="replaceValue"
          type="text"
          :placeholder="t('editor.find.placeholderReplace')"
          data-oc-find-input="replace"
          class="h-8 px-2.5 font-mono text-xs"
          :aria-label="t('editor.find.placeholderReplace')"
          @update:model-value="(value) => emit('update:replaceValue', String(value ?? ''))"
          @keydown="onReplaceKeydown"
        />
      </div>

      <IconButton
        size="xs"
        variant="outline"
        class="h-8 w-8 text-muted-foreground hover:text-foreground"
        :disabled="replaceCurrentDisabled"
        :title="t('editor.find.replaceCurrent')"
        :tooltip="t('editor.find.replaceCurrent')"
        :aria-label="t('editor.find.replaceCurrent')"
        @click="emit('replace')"
      >
        <RiArrowRightSLine class="h-4 w-4" />
      </IconButton>

      <IconButton
        size="xs"
        variant="outline"
        class="h-8 w-8 text-muted-foreground hover:text-foreground"
        :disabled="replaceAllDisabled"
        :title="t('editor.find.replaceAll')"
        :tooltip="t('editor.find.replaceAll')"
        :aria-label="t('editor.find.replaceAll')"
        @click="emit('replace-all')"
      >
        <RiListCheck3 class="h-4 w-4" />
      </IconButton>
    </div>
  </div>
</template>

<style scoped>
:global(.oc-monaco-find-match) {
  background: oklch(var(--primary) / 0.2);
  border-radius: 2px;
}

:global(.oc-monaco-find-match-current) {
  background: oklch(var(--primary) / 0.42);
  box-shadow: inset 0 0 0 1px oklch(var(--primary) / 0.72);
  border-radius: 2px;
}

:global(.dark .oc-monaco-find-match) {
  background: oklch(var(--primary) / 0.28);
}

:global(.dark .oc-monaco-find-match-current) {
  background: oklch(var(--primary) / 0.46);
  box-shadow: inset 0 0 0 1px oklch(var(--primary) / 0.78);
}

@media (max-width: 640px) {
  .oc-editor-find-bar {
    left: 0.5rem;
    right: 0.5rem;
    top: 0.5rem;
    width: auto;
  }
}
</style>
