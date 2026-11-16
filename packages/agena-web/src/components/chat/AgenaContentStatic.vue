<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import { highlightCodeToHtml } from '@/lib/highlight'
import type { ContentFormat } from '@/lib/content'
import AgenaDiffBlock from './AgenaDiffBlock.vue'

const props = defineProps<{
  text: string
  format?: ContentFormat
  wrap?: boolean
  followTail?: boolean
}>()
const root = ref<HTMLElement>()
const formatted = computed(() => {
  if (props.format === 'json') {
    try {
      return JSON.stringify(JSON.parse(props.text), null, 2)
    } catch {
      // Incomplete or mixed JSON output remains readable as text.
    }
  }
  return props.text
})
const highlighted = computed(() => highlightCodeToHtml(formatted.value, props.format === 'json' ? 'json' : 'text'))

onMounted(async () => {
  await nextTick()
  if (props.followTail && (!props.format || props.format === 'plain') && root.value)
    root.value.scrollTop = root.value.scrollHeight
})
</script>

<template>
  <div
    ref="root"
    class="max-h-[24rem] min-w-0 overflow-auto px-2 py-1"
    data-content-static
    :data-format="format || 'plain'"
    tabindex="0"
    data-transcript-keyboard-owner="true"
  >
    <MarkdownRenderer v-if="format === 'markdown'" :content="text" :stream="false" />
    <AgenaDiffBlock v-else-if="format === 'diff'" :diff="text" />
    <pre
      v-else
      class="m-0 font-mono text-xs leading-snug text-foreground/90"
      :class="wrap === false ? 'whitespace-pre' : 'whitespace-pre-wrap break-words'"
    ><code v-if="format === 'json' || format === 'code'" v-html="highlighted" /><template v-else>{{ text }}</template></pre>
  </div>
</template>
