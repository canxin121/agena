<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Dialog from '@/components/ui/Dialog.vue'
import Button from '@/components/ui/Button.vue'
import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import { askBtw } from '@/pages/chat/btwRequest'
import { useBtw } from '@/pages/chat/useBtw'

const props = defineProps<{ open: boolean; sessionId: string | null; initialQuestion?: string }>()
defineEmits<{ (event: 'update:open', open: boolean): void }>()
const { t } = useI18n()
const question = ref('')
const input = ref<HTMLTextAreaElement | null>(null)
const { loading, markdown, error, submit, stop } = useBtw(() => [props.open, props.sessionId], askBtw)
watch(
  () => [props.open, props.sessionId] as const,
  async ([open]) => {
    question.value = props.initialQuestion || ''
    if (!open) return
    if (question.value.trim()) void submit(question.value)
    await nextTick()
    if (props.open) input.value?.focus()
  },
  { flush: 'sync', immediate: true },
)
</script>

<template>
  <Dialog
    :open="open"
    :title="t('chat.btw.title')"
    :description="t('chat.btw.description')"
    max-width="max-w-3xl"
    mobile-fullscreen
    :body-scroll="false"
    @update:open="$emit('update:open', $event)"
  >
    <form class="flex min-h-0 flex-col gap-3" @submit.prevent="submit(question)">
      <textarea
        ref="input"
        v-model="question"
        :disabled="loading"
        :placeholder="t('chat.btw.placeholder')"
        :aria-label="t('chat.btw.placeholder')"
        rows="2"
        maxlength="16000"
        class="min-h-20 w-full resize-y rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-ring disabled:opacity-70"
        @keydown.enter.exact.prevent="submit(question)"
      />
      <div class="flex shrink-0 items-center gap-3">
        <Button v-if="loading" type="button" variant="outline" @click="stop">{{ t('chat.btw.stop') }}</Button>
        <Button v-else type="submit" :disabled="!question.trim() || !sessionId">{{ t('chat.btw.send') }}</Button>
        <span v-if="loading" role="status" class="text-xs text-muted-foreground">{{ t('chat.btw.loading') }}</span>
      </div>
      <div
        v-if="error"
        role="alert"
        class="border-l-2 border-destructive/60 pl-3 text-sm text-destructive [overflow-wrap:anywhere]"
      >
        {{ error }}
      </div>
      <div
        v-if="markdown"
        class="min-h-0 max-h-[55dvh] overflow-auto overscroll-contain border-t border-border/50 pt-3"
      >
        <MarkdownRenderer :content="markdown" mode="markdown" :stream="loading" />
      </div>
    </form>
  </Dialog>
</template>
