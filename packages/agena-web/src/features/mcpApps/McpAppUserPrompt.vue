<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import type { McpAppPrompt } from './api'

const props = defineProps<{ requestId: string; prompt: McpAppPrompt }>()
const emit = defineEmits<{ answer: [requestId: string, value: string | null] }>()
const { t } = useI18n()
const text = ref('')
const choices = computed(() => props.prompt.question.options?.filter((option) => !option.custom) ?? [])
const custom = computed(() => props.prompt.question.options?.find((option) => option.custom))
</script>

<template>
  <div class="shrink-0 border-t px-3 py-2 text-xs" role="group" :aria-label="prompt.title">
    <p class="font-medium">{{ prompt.title }}</p>
    <p class="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words">{{ prompt.question.question }}</p>
    <form
      v-if="custom || prompt.question.allow_custom"
      class="mt-2 flex flex-col items-start gap-2"
      @submit.prevent="text.trim() && emit('answer', requestId, text)"
    >
      <textarea
        v-model="text"
        :aria-label="custom?.label || prompt.title"
        :placeholder="custom?.label"
        class="max-h-32 w-full resize-y rounded-md border bg-background px-2 py-1 text-foreground focus-visible:ring"
        rows="3"
      />
      <Button type="submit" size="sm" :disabled="!text.trim()">{{ t('common.confirm') }}</Button>
    </form>
    <div class="mt-2 flex flex-wrap items-center gap-2">
      <Button
        v-for="choice in choices"
        :key="choice.label"
        size="sm"
        :title="choice.description"
        @click="emit('answer', requestId, choice.label)"
      >
        {{ choice.label }}
      </Button>
      <Button variant="ghost" size="sm" @click="emit('answer', requestId, null)">{{ t('common.cancel') }}</Button>
    </div>
  </div>
</template>
