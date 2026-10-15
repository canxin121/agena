<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftLine, RiFileTextLine, RiRefreshLine } from '@remixicon/vue'
import { useWorkspaceChanges } from '@/pages/chat/useWorkspaceChanges'
import SessionChip from './SessionChip.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'

const AgenaDiffBlock = defineAsyncComponent(() => import('./AgenaDiffBlock.vue'))
const props = defineProps<{ sessionId: string; directory: string; busy: boolean }>()
const { t } = useI18n()
const sessionId = computed(() => props.sessionId)
const {
  expanded,
  page,
  selected,
  status,
  current,
  files,
  total,
  hasChanges,
  recordingIncomplete,
  statusPending,
  diff,
  diffIdentity,
  visibleDiff,
  diffPending,
  select,
  refresh,
  moreDiff,
  moreDiffAvailable,
} = useWorkspaceChanges({ sessionId, busy: computed(() => props.busy) })
const detail = computed(() => visibleDiff.value?.files[0])
// The chip stays a number; the full sentence lives in the tooltip and panel.
const count = computed(() => (total.value === null ? '' : String(total.value)))
const summary = computed(() => {
  if (status.error.value) return t('chat.sessionWork.loadFailed')
  const recorded =
    total.value === null
      ? t('chat.sessionWork.loading')
      : total.value === 0
        ? t('chat.sessionWork.clean')
        : t('chat.sessionWork.files', { count: total.value })
  // The chip only exists for recorded edits; the caveat rides its tooltip.
  return recordingIncomplete.value ? `${recorded} · ${t('chat.sessionWork.recordingIncomplete')}` : recorded
})
const bodyRef = ref<HTMLElement | null>(null)
watch(
  [() => selected.value?.path, page, sessionId],
  () => {
    if (bodyRef.value) bodyRef.value.scrollTop = 0
  },
  { flush: 'post' },
)
</script>

<template>
  <SessionChip
    v-if="hasChanges"
    :title="t('chat.sessionWork.changes')"
    :label="count"
    :hint="summary"
    :open="expanded"
    :busy="statusPending"
    @update:open="expanded = $event"
  >
    <template #icon>
      <RiFileTextLine class="h-3 w-3 shrink-0" />
    </template>
    <template #actions>
      <IconButton
        class="h-7 w-7"
        :tooltip="t('chat.sessionWork.refresh')"
        :aria-label="t('chat.sessionWork.refresh')"
        :disabled="statusPending || diffPending"
        @click="refresh"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <div ref="bodyRef" class="min-h-0 min-w-0" :aria-busy="statusPending">
      <p v-if="status.error.value" role="alert" class="text-xs text-destructive [overflow-wrap:anywhere]">
        {{ status.error.value }}
      </p>
      <p v-if="!current && !status.error.value" class="text-xs text-muted-foreground">
        {{ t('chat.sessionWork.loading') }}
      </p>
      <p v-else-if="total === 0" class="text-xs text-muted-foreground">{{ t('chat.sessionWork.clean') }}</p>
      <template v-else-if="current">
        <template v-if="!selected">
          <div class="flex flex-col items-stretch gap-0.5">
            <button
              v-for="file in files"
              :key="file.path"
              type="button"
              class="flex min-h-8 min-w-0 items-center gap-2 rounded px-2 text-left font-mono text-xs hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
              :title="file.path"
              @click="select(file)"
            >
              <span class="flex shrink-0 items-center gap-1 font-mono">
                <span class="text-emerald-700 dark:text-emerald-400">+{{ file.added }}</span>
                <span class="text-rose-700 dark:text-rose-400">−{{ file.removed }}</span>
              </span>
              <span class="min-w-0 flex-1 truncate">{{ file.path }}</span>
            </button>
          </div>
          <div v-if="page || current?.has_more" class="my-2 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="ghost" :disabled="!page || statusPending" @click="page--">{{
              t('chat.sessionWork.previous')
            }}</Button>
            <Button size="sm" variant="ghost" :disabled="!current?.has_more || statusPending" @click="page++">{{
              t('chat.sessionWork.next')
            }}</Button>
            <span v-if="current?.files.length" class="text-xs text-muted-foreground">{{
              t('chat.sessionWork.pageRange', {
                start: current.offset + 1,
                end: current.offset + current.files.length,
                total: current.total_files,
              })
            }}</span>
          </div>
        </template>
        <div v-else class="min-w-0">
          <p class="mb-2 font-mono text-xs [overflow-wrap:anywhere]">{{ selected.path }}</p>
          <div class="mb-2 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="ghost" @click="selected = null">
              <RiArrowLeftLine class="mr-1 h-3.5 w-3.5" />{{ t('chat.sessionWork.backToFiles') }}
            </Button>
          </div>
          <p v-if="diff.error.value" role="alert" class="text-xs text-destructive [overflow-wrap:anywhere]">
            {{ diff.error.value }}
          </p>
          <p v-if="!detail && !diff.error.value" class="text-xs text-muted-foreground">
            {{ t(diffPending ? 'chat.sessionWork.loading' : 'chat.sessionWork.noDiff') }}
          </p>
          <template v-else-if="detail">
            <p v-if="detail.operation_history" class="mb-2 text-xs text-muted-foreground">
              {{ t('chat.sessionWork.operationHistory') }}
            </p>
            <div v-for="op in detail.operations" :key="op.part_id" class="mb-1.5 min-w-0">
              <p class="text-xs text-muted-foreground [overflow-wrap:anywhere]">
                #{{ op.part_id }} · {{ op.tool }} · {{ op.kind
                }}<span v-if="op.from_path"> · {{ op.from_path }} → {{ selected.path }}</span>
              </p>
              <p
                v-if="op.before_sha256 || op.after_sha256"
                class="font-mono text-xs text-muted-foreground [overflow-wrap:anywhere]"
              >
                {{ op.before_sha256 || '?' }} → {{ op.after_sha256 || '?' }}
              </p>
              <p v-if="op.diff_scope === 'operation'" class="text-xs text-muted-foreground">
                {{ t('chat.sessionWork.operationDiff') }}
              </p>
              <AgenaDiffBlock v-if="op.diff" :key="`${diffIdentity}:${op.part_id}`" :diff="op.diff" />
              <p v-else class="text-xs text-muted-foreground">
                {{ op.diff_unavailable_reason || t('chat.sessionWork.noDiff') }}
              </p>
              <p v-if="op.diff_truncated" class="text-xs text-muted-foreground">
                {{ t('chat.sessionWork.recordedTruncated') }}
              </p>
            </div>
            <Button
              v-if="detail.operations.some((op) => op.diff_truncated)"
              size="sm"
              variant="ghost"
              :disabled="diffPending || !moreDiffAvailable"
              @click="moreDiff"
              >{{ t('chat.sessionWork.moreDiff') }}</Button
            >
          </template>
        </div>
      </template>
    </div>
  </SessionChip>
</template>
