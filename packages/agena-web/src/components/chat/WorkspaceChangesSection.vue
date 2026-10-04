<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useChatStore } from '@/stores/chat'
import { gitJson } from '@/lib/gitApi'
import type { GitStatusFile, GitStatusResponse, GitDiffResponse } from '@/types/git'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import SessionSection from './SessionSection.vue'
import Button from '@/components/ui/Button.vue'

const AgenaDiffBlock = defineAsyncComponent(() => import('./AgenaDiffBlock.vue'))
const props = defineProps<{ sessionId: string; directory: string; busy: boolean }>()
const { t } = useI18n()
const chat = useChatStore()
const directory = computed(
  () => chat.getSessionExecution(props.sessionId)?.effective_workspace_root?.trim() || props.directory,
)
const expanded = ref(false)
const page = ref(0)
const selected = ref<GitStatusFile | null>(null)
const staged = ref(false)
const diffLimit = ref(256 * 1024)
// Page replacement keeps both network and rendered list bounded, including
// repositories with thousands of untracked files. Counts never require stats.
const statusKey = computed(() => (directory.value ? JSON.stringify([directory.value, expanded.value, page.value]) : ''))
const status = useVisibleResource<GitStatusResponse>({
  key: statusKey,
  interval: () => (props.busy || expanded.value ? 5000 : 30_000),
  load: (key, signal) => {
    const [directory, open, index] = JSON.parse(key) as [string, boolean, number]
    return gitJson(
      'status',
      directory,
      { summary: !open, offset: index * 40, limit: 40, includeDiffStats: false },
      { signal },
    )
  },
})
const diffKey = computed(() =>
  expanded.value && selected.value
    ? JSON.stringify([
        directory.value,
        selected.value.path,
        staged.value,
        (staged.value ? selected.value.indexOldPath : selected.value.workingOldPath) || null,
      ])
    : '',
)
const diff = useVisibleResource<GitDiffResponse>({
  key: diffKey,
  interval: () => (props.busy ? 5000 : 30_000),
  load: (key, signal) => {
    const [directory, path, index, oldPath] = JSON.parse(key) as [string, string, boolean, string | null]
    return gitJson(
      'diff',
      directory,
      { path, staged: index, contextLines: 3, maxBytes: diffLimit.value, oldPath },
      { signal },
    )
  },
})
const total = ref<number | null>(null)
watch(status.data, (value) => {
  if (!value) return
  total.value = value.totalFiles
  if (expanded.value && page.value && !value.files.length) page.value = 0
  if (
    selected.value &&
    page.value === 0 &&
    !value.hasMore &&
    expanded.value &&
    !value.files.some((file) => file.path === selected.value?.path)
  )
    selected.value = null
})
watch(
  directory,
  () => {
    total.value = null
    page.value = 0
    selected.value = null
  },
  { flush: 'sync' },
)
function select(file: GitStatusFile) {
  diffLimit.value = 256 * 1024
  selected.value = selected.value?.path === file.path ? null : file
  staged.value = Boolean(file.index.trim() && !file.workingDir.trim())
}
function refresh() {
  void status.refresh()
  void diff.refresh()
}
function moreDiff() {
  diffLimit.value += 256 * 1024
  void diff.refresh()
}
</script>

<template>
  <SessionSection
    v-if="directory"
    v-model:expanded="expanded"
    :title="t('chat.sessionWork.changes')"
    :summary="total === null ? '' : t('chat.sessionWork.files', { count: total })"
    :busy="status.loading.value"
  >
    <template #actions>
      <Button size="sm" variant="ghost" :disabled="status.loading.value" @click="refresh">{{
        t('chat.sessionWork.refresh')
      }}</Button>
    </template>
    <div class="max-h-[min(50dvh,32rem)] min-w-0 overflow-auto overscroll-contain">
      <p class="mb-2 break-all text-xs text-muted-foreground">
        {{ t('chat.sessionWork.workspaceScope') }} · {{ directory }}
      </p>
      <p v-if="status.error.value" role="alert" class="mb-2 text-xs text-destructive">{{ status.error.value }}</p>
      <p v-else-if="!total && !status.loading.value" class="text-xs text-muted-foreground">
        {{ t('chat.sessionWork.clean') }}
      </p>
      <div class="flex flex-col items-stretch gap-0.5">
        <button
          v-for="file in status.data.value?.files || []"
          :key="file.path"
          type="button"
          class="flex min-h-8 min-w-0 items-center gap-2 rounded px-2 text-left font-mono text-xs hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
          :class="{ 'bg-secondary/60': selected?.path === file.path }"
          :aria-expanded="selected?.path === file.path"
          @click="select(file)"
        >
          <span class="w-6 shrink-0 whitespace-pre text-primary" :title="t('chat.sessionWork.indexWorking')"
            >{{ file.index }}{{ file.workingDir }}</span
          >
          <span class="min-w-0 truncate" :title="file.path">{{ file.path }}</span>
        </button>
      </div>
      <div v-if="page || status.data.value?.hasMore" class="my-2 flex items-center gap-2">
        <Button size="sm" variant="ghost" :disabled="!page || status.loading.value" @click="page--">{{
          t('chat.sessionWork.previous')
        }}</Button>
        <Button
          size="sm"
          variant="ghost"
          :disabled="!status.data.value?.hasMore || status.loading.value"
          @click="page++"
          >{{ t('chat.sessionWork.next') }}</Button
        >
        <span class="text-xs text-muted-foreground"
          >{{ page * 40 + 1 }}–{{ page * 40 + (status.data.value?.files.length || 0) }}</span
        >
      </div>
      <div v-if="selected" class="mt-2 min-w-0 border-t border-border/50 pt-2">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <Button size="sm" :variant="staged ? 'ghost' : 'secondary'" @click="staged = false">{{
            t('chat.sessionWork.working')
          }}</Button>
          <Button size="sm" :variant="staged ? 'secondary' : 'ghost'" @click="staged = true">{{
            t('chat.sessionWork.staged')
          }}</Button>
          <Button size="sm" variant="ghost" @click="selected = null">{{ t('chat.sessionWork.close') }}</Button>
        </div>
        <p v-if="diff.error.value" role="alert" class="text-xs text-destructive">{{ diff.error.value }}</p>
        <p v-else-if="!diff.data.value?.diff" class="text-xs text-muted-foreground">
          {{ t(diff.loading.value ? 'chat.sessionWork.loading' : 'chat.sessionWork.noDiff') }}
        </p>
        <AgenaDiffBlock v-else :key="diffKey" :diff="diff.data.value.diff" />
        <Button
          v-if="diff.data.value?.truncated"
          size="sm"
          variant="ghost"
          :disabled="diff.loading.value"
          @click="moreDiff"
          >{{ t('chat.sessionWork.moreDiff') }}</Button
        >
      </div>
    </div>
  </SessionSection>
</template>
