<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftLine, RiRefreshLine } from '@remixicon/vue'
import { useChatStore } from '@/stores/chat'
import { useWorkspaceChanges } from '@/pages/chat/useWorkspaceChanges'
import SessionSection from './SessionSection.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'

const AgenaDiffBlock = defineAsyncComponent(() => import('./AgenaDiffBlock.vue'))
const props = defineProps<{ sessionId: string; directory: string; busy: boolean }>()
const { t } = useI18n()
const chat = useChatStore()
const directory = computed(
  () => chat.getSessionExecution(props.sessionId)?.effective_workspace_root?.trim() || props.directory.trim(),
)
const {
  expanded,
  page,
  selected,
  staged,
  status,
  current,
  files,
  total,
  notRepository,
  statusPending,
  hasStaged,
  hasWorking,
  diff,
  diffIdentity,
  visibleDiff,
  diffPending,
  select,
  refresh,
  moreDiff,
} = useWorkspaceChanges({ directory, busy: computed(() => props.busy) })
const summary = computed(() => {
  if (status.error.value) return t('chat.sessionWork.loadFailed')
  if (notRepository.value) return t('chat.sessionWork.notRepository')
  if (total.value === null) return t('chat.sessionWork.loading')
  return total.value === 0 ? t('chat.sessionWork.clean') : t('chat.sessionWork.files', { count: total.value })
})
const bodyRef = ref<HTMLElement | null>(null)
watch(
  [() => selected.value?.path, page, directory],
  () => {
    if (bodyRef.value) bodyRef.value.scrollTop = 0
  },
  { flush: 'post' },
)
const oldPath = computed(() => (staged.value ? selected.value?.indexOldPath : selected.value?.workingOldPath))
</script>

<template>
  <SessionSection
    v-if="directory"
    v-model:expanded="expanded"
    :title="t('chat.sessionWork.changes')"
    :summary="summary"
    :busy="statusPending"
  >
    <template #actions>
      <IconButton
        class="h-8 w-8"
        :tooltip="t('chat.sessionWork.refresh')"
        :aria-label="t('chat.sessionWork.refresh')"
        :disabled="statusPending || diffPending"
        @click="refresh"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <div
      ref="bodyRef"
      class="max-h-[min(50dvh,32rem)] min-w-0 overflow-auto overscroll-contain"
      :aria-busy="statusPending"
    >
      <div class="mb-2 text-xs text-muted-foreground">
        <p>{{ t('chat.sessionWork.workspaceScope') }}</p>
        <p class="mt-1 font-mono [overflow-wrap:anywhere]">{{ directory }}</p>
      </div>
      <p v-if="status.error.value" role="alert" class="text-xs text-destructive [overflow-wrap:anywhere]">
        {{ status.error.value }}
      </p>
      <p v-else-if="notRepository" class="text-xs text-muted-foreground">{{ t('chat.sessionWork.notRepository') }}</p>
      <p v-else-if="!current" class="text-xs text-muted-foreground">{{ t('chat.sessionWork.loading') }}</p>
      <p v-else-if="total === 0" class="text-xs text-muted-foreground">{{ t('chat.sessionWork.clean') }}</p>
      <template v-else>
        <template v-if="!selected">
          <div class="flex flex-col items-stretch gap-0.5">
            <button
              v-for="file in files"
              :key="file.path"
              type="button"
              class="flex min-h-8 min-w-0 items-center gap-2 rounded px-2 text-left font-mono text-xs hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
              :title="[file.workingOldPath || file.indexOldPath, file.path].filter(Boolean).join(' → ')"
              @click="select(file)"
            >
              <span class="w-6 shrink-0 whitespace-pre text-primary" :title="t('chat.sessionWork.indexWorking')"
                >{{ file.index || ' ' }}{{ file.workingDir || ' ' }}</span
              >
              <span class="min-w-0 flex-1 truncate">{{ file.path }}</span>
            </button>
          </div>
          <div v-if="page || current?.hasMore" class="my-2 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="ghost" :disabled="!page || statusPending" @click="page--">{{
              t('chat.sessionWork.previous')
            }}</Button>
            <Button size="sm" variant="ghost" :disabled="!current?.hasMore || statusPending" @click="page++">{{
              t('chat.sessionWork.next')
            }}</Button>
            <span v-if="current?.files.length" class="text-xs text-muted-foreground">{{
              t('chat.sessionWork.pageRange', {
                start: current.offset + 1,
                end: current.offset + current.files.length,
                total: current.totalFiles,
              })
            }}</span>
          </div>
        </template>
        <div v-else class="min-w-0">
          <p v-if="diff.error.value || !visibleDiff?.diff" class="mb-2 font-mono text-xs [overflow-wrap:anywhere]">
            <span v-if="oldPath" class="text-muted-foreground">{{ oldPath }} → </span>{{ selected.path }}
          </p>
          <div class="mb-2 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="ghost" @click="selected = null">
              <RiArrowLeftLine class="mr-1 h-3.5 w-3.5" />{{ t('chat.sessionWork.backToFiles') }}
            </Button>
            <Button
              size="sm"
              :variant="staged ? 'ghost' : 'secondary'"
              :disabled="!hasWorking"
              :aria-pressed="!staged"
              @click="staged = false"
              >{{ t('chat.sessionWork.working') }}</Button
            >
            <Button
              size="sm"
              :variant="staged ? 'secondary' : 'ghost'"
              :disabled="!hasStaged"
              :aria-pressed="staged"
              @click="staged = true"
              >{{ t('chat.sessionWork.staged') }}</Button
            >
          </div>
          <p v-if="diff.error.value" role="alert" class="text-xs text-destructive [overflow-wrap:anywhere]">
            {{ diff.error.value }}
          </p>
          <p v-else-if="!visibleDiff?.diff" class="text-xs text-muted-foreground">
            {{ t(diffPending ? 'chat.sessionWork.loading' : 'chat.sessionWork.noDiff') }}
          </p>
          <AgenaDiffBlock v-else :key="diffIdentity" :diff="visibleDiff.diff" />
          <Button v-if="visibleDiff?.truncated" size="sm" variant="ghost" :disabled="diffPending" @click="moreDiff">{{
            t('chat.sessionWork.moreDiff')
          }}</Button>
        </div>
      </template>
    </div>
  </SessionSection>
</template>
