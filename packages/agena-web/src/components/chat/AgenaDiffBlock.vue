<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { transcriptDiffFiles } from '@/pages/chat/transcriptDiff'
import { highlightCodeToHtml } from '@/lib/highlight'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useToastsStore } from '@/stores/toasts'

const props = defineProps<{ diff: string }>()
const { t } = useI18n()
const expanded = ref(false)
const copied = ref(false)
const toasts = useToastsStore()
watch(
  () => props.diff,
  () => {
    copied.value = false
  },
)
async function copyDiff() {
  copied.value = await copyTextToClipboard(props.diff)
  if (!copied.value) toasts.push('error', t('common.copyFailed'))
}
const files = computed(() => transcriptDiffFiles(props.diff))
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
const previewLimit = 80
function highlighted(text: string, path: string) {
  return highlightCodeToHtml(text, languages[path.split('.').pop() || ''] || 'text')
}
const totalRows = computed(() => files.value.reduce((sum, file) => sum + file.rows.length, 0))
const visibleFiles = computed(() => {
  let remaining = expanded.value ? Infinity : previewLimit
  return files.value.map((file) => {
    const rows = file.rows.slice(0, Math.max(0, remaining))
    remaining -= rows.length
    return { ...file, rows: rows.map((row) => ({ ...row, html: highlighted(row.text, file.path) || ' ' })) }
  })
})
</script>

<template>
  <div class="min-w-0 space-y-1.5" data-transcript-diff>
    <section
      v-for="(file, index) in visibleFiles"
      :key="`${file.path}:${index}`"
      class="overflow-hidden rounded-md border border-border/60"
    >
      <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1 bg-muted/30 px-3 py-1 font-mono text-xs">
        <span class="text-muted-foreground" :title="t(`chat.toolDetails.diffChange${file.change}`)">{{
          file.change
        }}</span>
        <span class="min-w-0 break-all font-medium">
          <span v-if="file.oldPath && file.oldPath !== file.path" class="text-muted-foreground"
            >{{ file.oldPath }} → </span
          >{{ file.path || t('chat.toolDetails.diff') }}
        </span>
        <span class="text-emerald-700 dark:text-emerald-400">+{{ file.additions }}</span>
        <span class="text-rose-700 dark:text-rose-400">−{{ file.deletions }}</span>
      </div>
      <div class="overflow-x-auto" tabindex="0" :aria-label="file.path || t('chat.toolDetails.diff')">
        <div class="min-w-full w-max font-mono text-xs leading-[1.4]">
          <div
            v-for="(row, rowIndex) in file.rows"
            :key="rowIndex"
            class="flex"
            :class="{
              'bg-emerald-500/10': row.kind === 'added',
              'bg-rose-500/10': row.kind === 'removed',
              'bg-muted/30 text-muted-foreground': row.kind === 'hunk' || row.kind === 'note',
            }"
            :data-diff-line="row.kind"
          >
            <span class="w-12 shrink-0 select-none pr-2 text-right text-muted-foreground/70" aria-hidden="true">{{
              row.kind === 'removed' ? row.oldLine : row.newLine
            }}</span>
            <span
              class="w-5 shrink-0 select-none text-center"
              aria-hidden="true"
              :class="
                row.kind === 'added'
                  ? 'text-emerald-700 dark:text-emerald-400'
                  : row.kind === 'removed'
                    ? 'text-rose-700 dark:text-rose-400'
                    : ''
              "
              >{{ row.kind === 'added' ? '+' : row.kind === 'removed' ? '−' : '' }}</span
            >
            <code
              v-if="row.kind !== 'hunk' && row.kind !== 'note'"
              class="block whitespace-pre pr-3"
              v-html="row.html"
            />
            <span v-else class="whitespace-pre pr-3">{{ row.text }}</span>
          </div>
        </div>
      </div>
    </section>
    <div class="flex flex-wrap items-center gap-3">
      <button
        type="button"
        class="min-h-8 rounded px-2 text-xs text-muted-foreground hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
        @click="copyDiff"
      >
        {{ copied ? t('common.copied') : t('chat.toolDetails.copyDiff') }}
      </button>
      <button
        v-if="totalRows > previewLimit"
        type="button"
        class="flex min-h-8 items-center gap-2 rounded px-2 text-xs text-primary hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
        :aria-expanded="expanded"
        @click="expanded = !expanded"
      >
        {{
          expanded
            ? t('chat.toolDetails.collapseDiff')
            : t('chat.toolDetails.expandDiff', { count: totalRows - previewLimit })
        }}
      </button>
    </div>
  </div>
</template>
