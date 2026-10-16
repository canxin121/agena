<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { DiffFile, DiffRow } from '@/pages/chat/transcriptDiff'
import { highlightCodeToHtml } from '@/lib/highlight'

const props = defineProps<{ files: DiffFile[]; label: string }>()
const rowHeight = 22
const viewportHeight = 440
const overscan = 8
const scrollTop = ref(0)
const viewport = ref<HTMLElement | null>(null)
let scrollFrame = 0
type Entry = { file: DiffFile; row?: DiffRow }
const entries = computed(() =>
  props.files.flatMap((file): Entry[] => [{ file }, ...file.rows.map((row) => ({ file, row }))]),
)
const start = computed(() =>
  Math.max(0, Math.min(entries.value.length - 1, Math.floor(scrollTop.value / rowHeight) - overscan)),
)
const languages: Record<string, string> = {
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  py: 'python',
  sh: 'bash',
  md: 'markdown',
  html: 'xml',
  vue: 'xml',
  css: 'css',
  go: 'go',
  json: 'json',
}
const visible = computed(() =>
  entries.value
    .slice(start.value, start.value + Math.ceil(viewportHeight / rowHeight) + overscan * 2)
    .map((entry, index) => ({
      ...entry,
      index: start.value + index,
      html: entry.row
        ? highlightCodeToHtml(entry.row.text, languages[entry.file.path.split('.').pop() || ''] || 'text')
        : '',
    })),
)
function onScroll() {
  if (scrollFrame) return
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = 0
    scrollTop.value = viewport.value?.scrollTop || 0
  })
}
onBeforeUnmount(() => {
  if (scrollFrame) cancelAnimationFrame(scrollFrame)
})
</script>

<template>
  <div
    ref="viewport"
    class="overflow-auto rounded-md border border-border/60 font-mono text-xs"
    :style="{ maxHeight: `${viewportHeight}px` }"
    tabindex="0"
    data-transcript-keyboard-owner="true"
    :aria-label="label"
    data-diff-viewport
    @scroll.passive="onScroll"
  >
    <div class="relative min-w-full w-max" :style="{ height: `${entries.length * rowHeight}px` }">
      <div :style="{ paddingTop: `${start * rowHeight}px` }">
        <div
          v-for="entry in visible"
          :key="entry.index"
          class="flex items-center whitespace-pre"
          :style="{ height: `${rowHeight}px` }"
          :data-diff-line="entry.row?.kind"
          :title="entry.file.path"
          :class="{
            'bg-muted/30': !entry.row || entry.row.kind === 'hunk' || entry.row.kind === 'note',
            'bg-emerald-500/10': entry.row?.kind === 'added',
            'bg-rose-500/10': entry.row?.kind === 'removed',
          }"
        >
          <template v-if="!entry.row">
            <span class="px-3 text-muted-foreground">{{ entry.file.change }}</span>
            <span class="font-medium"
              >{{ entry.file.oldPath && entry.file.oldPath !== entry.file.path ? `${entry.file.oldPath} → ` : ''
              }}{{ entry.file.path }}</span
            >
            <span class="px-3 text-emerald-700 dark:text-emerald-400">+{{ entry.file.additions }}</span>
            <span class="pr-3 text-rose-700 dark:text-rose-400">−{{ entry.file.deletions }}</span>
          </template>
          <template v-else>
            <span class="w-12 shrink-0 select-none pr-2 text-right text-muted-foreground/70" aria-hidden="true">{{
              entry.row.kind === 'removed' ? entry.row.oldLine : entry.row.newLine
            }}</span>
            <span class="w-5 shrink-0 select-none text-center" aria-hidden="true">{{
              entry.row.kind === 'added' ? '+' : entry.row.kind === 'removed' ? '−' : ''
            }}</span>
            <code class="block pr-3" v-html="entry.html || ' '" />
          </template>
        </div>
      </div>
    </div>
  </div>
</template>
