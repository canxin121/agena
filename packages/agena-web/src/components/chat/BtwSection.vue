<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowDownSLine, RiArrowRightSLine, RiCloseLine, RiFileCopyLine, RiLoader4Line } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useBtwStore } from '@/stores/btw'

const MarkdownRenderer = defineAsyncComponent(() => import('@/components/markdown/MarkdownRenderer.vue'))
const props = defineProps<{ sessionId: string }>()
const { t } = useI18n()
const btw = useBtwStore()
const state = computed(() => btw.sessions.get(props.sessionId))
const loading = computed(() => state.value?.exchanges.some((entry) => entry.status === 'running') ?? false)
const input = ref<HTMLTextAreaElement | null>(null)

watch(
  () => state.value?.focusRequest,
  async (request) => {
    if (!request) return
    const sessionId = props.sessionId
    await nextTick()
    if (sessionId === props.sessionId && state.value?.focusRequest === request) {
      state.value.focusRequest = 0
      input.value?.focus({ preventScroll: true })
    }
  },
  { immediate: true },
)

function handleKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
    event.preventDefault()
    void btw.submit(props.sessionId)
  }
}
</script>

<template>
  <section
    v-if="state"
    class="my-3 min-w-0 rounded-lg border border-border/60 bg-secondary/10"
    :aria-label="t('chat.btw.title')"
    data-transcript-chrome="true"
  >
    <header class="flex min-h-9 items-center gap-2 px-2">
      <button
        type="button"
        class="flex min-h-8 min-w-0 flex-1 items-center gap-2 rounded px-1 text-left text-xs hover:bg-secondary/50"
        :aria-expanded="state.expanded"
        @click="state.expanded = !state.expanded"
      >
        <component :is="state.expanded ? RiArrowDownSLine : RiArrowRightSLine" class="h-4 w-4 shrink-0" />
        <span class="font-mono font-semibold">BTW</span>
        <span class="truncate text-muted-foreground">{{ t('chat.btw.inlineHint') }}</span>
        <RiLoader4Line v-if="loading" class="h-3.5 w-3.5 shrink-0 animate-spin" :aria-label="t('chat.btw.loading')" />
        <span v-else-if="state.exchanges.length" class="text-muted-foreground">{{ state.exchanges.length }}</span>
      </button>
      <IconButton
        class="h-7 w-7"
        :tooltip="t('chat.btw.clear')"
        :aria-label="t('chat.btw.clear')"
        @click="btw.clear(sessionId)"
      >
        <RiCloseLine class="h-3.5 w-3.5" />
      </IconButton>
    </header>
    <div v-if="state.expanded" class="space-y-3 border-t border-border/50 px-3 py-3">
      <article v-for="entry in state.exchanges" :key="entry.id" class="min-w-0">
        <div class="flex items-start gap-2">
          <button
            type="button"
            class="flex min-h-7 min-w-0 flex-1 items-start gap-1.5 text-left text-sm"
            :aria-expanded="entry.expanded"
            @click="entry.expanded = !entry.expanded"
          >
            <component
              :is="entry.expanded ? RiArrowDownSLine : RiArrowRightSLine"
              class="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
            />
            <span :class="entry.expanded ? 'whitespace-pre-wrap [overflow-wrap:anywhere]' : 'truncate'">{{
              entry.question
            }}</span>
          </button>
          <IconButton
            v-if="entry.markdown"
            class="h-7 w-7"
            :tooltip="t('common.copy')"
            :aria-label="t('common.copy')"
            @click="copyTextToClipboard(entry.markdown)"
          >
            <RiFileCopyLine class="h-3.5 w-3.5" />
          </IconButton>
        </div>
        <div v-if="entry.expanded" class="mt-2 min-w-0 border-l-2 border-border/60 pl-3">
          <MarkdownRenderer
            v-if="entry.markdown"
            :content="entry.markdown"
            mode="markdown"
            :stream="entry.status === 'running'"
          />
          <p v-if="entry.error" role="alert" class="text-sm text-destructive [overflow-wrap:anywhere]">
            {{ entry.error }}
          </p>
          <p v-if="entry.status === 'running'" role="status" class="text-xs text-muted-foreground">
            {{ t('chat.btw.loading') }}
          </p>
          <p v-else-if="entry.status === 'stopped'" class="text-xs text-muted-foreground">
            {{ t('chat.btw.stopped') }}
          </p>
        </div>
      </article>
      <form class="flex items-end gap-2" @submit.prevent="btw.submit(sessionId)">
        <textarea
          ref="input"
          v-model="state.draft"
          :placeholder="t('chat.btw.placeholder')"
          :aria-label="t('chat.btw.placeholder')"
          rows="2"
          maxlength="16000"
          class="min-h-16 min-w-0 flex-1 resize-y rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-ring"
          @keydown="handleKeydown"
        />
        <Button v-if="loading" type="button" size="sm" variant="outline" @click="btw.stop(sessionId)">{{
          t('chat.btw.stop')
        }}</Button>
        <Button v-else type="submit" size="sm" variant="outline" :disabled="!state.draft.trim()">{{
          t('chat.btw.send')
        }}</Button>
      </form>
    </div>
  </section>
</template>
