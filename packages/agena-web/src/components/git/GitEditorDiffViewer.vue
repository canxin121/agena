<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, ref, watch } from 'vue'
import { RiFileTextLine, RiLoader4Line, RiSearchLine, RiTextWrap } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { apiJson } from '@/lib/api'
import { createRevalidator } from '@/lib/revalidation'
import { confirmAction } from '@/lib/appConfirm'
import IconButton from '@/components/ui/IconButton.vue'
import { buildUnifiedDiffModel } from '@/features/git/diff/unifiedDiff'
import { useUiStore } from '@/stores/ui'
import type { GitCommitDiffResponse, GitCommitFileContentResponse, GitDiffMeta, GitDiffResponse } from '@/types/git'

const { t } = useI18n()
const ui = useUiStore()
const MonacoDiffEditor = defineAsyncComponent(() => import('@/components/MonacoDiffEditor.vue'))
const visible = usePaneVisibility()

type DiffHunkView = {
  id: string
  header: string
  range: string
  anchorLine: number
  oldStart: number
  oldCount: number
  newStart: number
  newCount: number
  additions: number
  deletions: number
  patch: string
  patchReady: boolean
}

type EditorHunkAction = {
  id: string
  anchorLine: number
  oldStart: number
  oldCount: number
  newStart: number
  newCount: number
  additions: number
  deletions: number
  stageEnabled: boolean
  unstageEnabled: boolean
  discardEnabled: boolean
  disabled: boolean
}

type HunkActionMode = 'stage' | 'unstage' | 'discard'

const props = defineProps<{
  directory: string | null
  path: string | null
  staged?: boolean
  commit?: string | null
  parentCommit?: string | null
  onStageHunk?: (patch: string) => void | Promise<void>
  onUnstageHunk?: (patch: string) => void | Promise<void>
  onDiscardHunk?: (patch: string) => void | Promise<void>
  onOpenFile?: (path: string) => void | Promise<void>
  onRevealFile?: (path: string) => void | Promise<void>
}>()

const loading = ref(false)
const error = ref<string | null>(null)
const wrapLines = ref(true)

const diffText = ref('')
const diffMeta = ref<GitDiffMeta | null>(null)
const original = ref('')
const modified = ref('')
const loadedModelScope = ref('')
const loadedLanguagePath = ref('')

// Keep current content visible while switching files.
const staleDiffText = ref('')
const staleDiffMeta = ref<GitDiffMeta | null>(null)
const staleOriginal = ref('')
const staleModified = ref('')

const activeHunkAction = ref<{ hunkId: string; mode: HunkActionMode } | null>(null)

let loadSeq = 0
let activeAbort: AbortController | null = null

function isDataImageUrl(value: string): boolean {
  return typeof value === 'string' && value.startsWith('data:image/')
}

const normalizedPath = computed(() => (props.path || '').trim())
const normalizedDirectory = computed(() => (props.directory || '').trim())
const diffScope = computed(() => (props.staged ? 'staged' : 'working'))
const commitScope = computed(() => {
  const commit = (props.commit || '').trim()
  const parent = (props.parentCommit || '').trim()
  if (!commit) return 'workspace'
  return `commit:${commit}:${parent || 'root'}`
})

const originalModelPath = computed(() => {
  return `git-diff:original:${loadedModelScope.value}`
})

const modifiedModelPath = computed(() => {
  return `git-diff:modified:${loadedModelScope.value}`
})
const requestedModelScope = computed(() =>
  JSON.stringify([normalizedDirectory.value, diffScope.value, commitScope.value, normalizedPath.value]),
)

const displayOriginal = computed(() => (loading.value ? original.value || staleOriginal.value : original.value))
const displayModified = computed(() => (loading.value ? modified.value || staleModified.value : modified.value))

const isImageDiff = computed(() => isDataImageUrl(displayOriginal.value) || isDataImageUrl(displayModified.value))
const leftLabel = computed(() => (props.staged ? 'HEAD' : t('git.ui.diffViewer.labels.index')))
const rightLabel = computed(() =>
  props.staged ? t('git.ui.diffViewer.labels.index') : t('git.ui.diffViewer.labels.workingTree'),
)
const canOpenFile = computed(() => Boolean(props.onOpenFile) && Boolean(normalizedPath.value))
const canRevealFile = computed(() => Boolean(props.onRevealFile) && Boolean(normalizedPath.value))
const actionsScoped = computed(() => loadedModelScope.value === requestedModelScope.value)
const actionsReady = computed(() => !loading.value && actionsScoped.value)
const canStageHunk = computed(() => actionsScoped.value && !props.staged && Boolean(props.onStageHunk))
const canUnstageHunk = computed(() => actionsScoped.value && Boolean(props.staged) && Boolean(props.onUnstageHunk))
const canDiscardHunk = computed(() => actionsScoped.value && !props.staged && Boolean(props.onDiscardHunk))
const hasAnyHunkAction = computed(() => canStageHunk.value || canUnstageHunk.value || canDiscardHunk.value)
const isAnyActionBusy = computed(() => Boolean(activeHunkAction.value))

const activeDiff = computed(() => (loading.value ? diffText.value || staleDiffText.value : diffText.value))
const activeDiffMeta = computed(() => (loading.value ? diffMeta.value || staleDiffMeta.value : diffMeta.value))
const parsedDiff = computed(() => buildUnifiedDiffModel(activeDiff.value, activeDiffMeta.value))

const hunks = computed<DiffHunkView[]>(() => {
  return parsedDiff.value.hunks.map((hunk) => ({
    id: hunk.id,
    header: hunk.header,
    range: hunk.range,
    anchorLine: hunk.anchorLine,
    oldStart: hunk.oldStart,
    oldCount: hunk.oldCount,
    newStart: hunk.newStart,
    newCount: hunk.newCount,
    additions: hunk.additions,
    deletions: hunk.deletions,
    patch: hunk.patch,
    patchReady: hunk.patchReady,
  }))
})

const hunkById = computed(() => {
  const map = new Map<string, DiffHunkView>()
  for (const hunk of hunks.value) {
    map.set(hunk.id, hunk)
  }
  return map
})

const editorHunkActions = computed<EditorHunkAction[]>(() =>
  hunks.value.map((hunk) => ({
    id: hunk.id,
    anchorLine: hunk.anchorLine,
    oldStart: hunk.oldStart,
    oldCount: hunk.oldCount,
    newStart: hunk.newStart,
    newCount: hunk.newCount,
    additions: hunk.additions,
    deletions: hunk.deletions,
    stageEnabled: canStageHunk.value,
    unstageEnabled: canUnstageHunk.value,
    discardEnabled: canDiscardHunk.value,
    disabled: !hunk.patchReady,
  })),
)

const firstChangedLine = computed<number | null>(() => {
  for (const hunk of hunks.value) {
    const anchor = Number(hunk.anchorLine)
    if (Number.isFinite(anchor) && anchor > 0) return Math.floor(anchor)

    const fallback = Number(hunk.newStart || hunk.oldStart)
    if (Number.isFinite(fallback) && fallback > 0) return Math.floor(fallback)
  }
  return null
})

function openImageDiffPreview(preferred: 'left' | 'right') {
  const items: Array<{ src: string; title: string; alt: string; key: string }> = []
  const leftSrc = isDataImageUrl(displayOriginal.value) ? String(displayOriginal.value || '').trim() : ''
  const rightSrc = isDataImageUrl(displayModified.value) ? String(displayModified.value || '').trim() : ''

  if (leftSrc) {
    const label = String(leftLabel.value || 'left')
    items.push({ src: leftSrc, title: label, alt: label, key: 'left' })
  }
  if (rightSrc) {
    const label = String(rightLabel.value || 'right')
    items.push({ src: rightSrc, title: label, alt: label, key: 'right' })
  }
  if (!items.length) return

  const preferredKey = preferred === 'left' ? 'left' : 'right'
  const preferredIndex = items.findIndex((item) => item.key === preferredKey)
  ui.openImageViewer(items, preferredIndex >= 0 ? preferredIndex : 0)
}

function resetState() {
  loading.value = false
  error.value = null
  diffText.value = ''
  diffMeta.value = null
  original.value = ''
  modified.value = ''
  loadedModelScope.value = ''
  loadedLanguagePath.value = ''
  staleDiffText.value = ''
  staleDiffMeta.value = null
  staleOriginal.value = ''
  staleModified.value = ''
  activeHunkAction.value = null
}

async function runHunkAction(hunk: DiffHunkView, mode: HunkActionMode) {
  if (!actionsReady.value) return
  if (!hunk.patchReady || !hunk.patch) return
  if (isAnyActionBusy.value) return

  const apply = mode === 'stage' ? props.onStageHunk : mode === 'unstage' ? props.onUnstageHunk : props.onDiscardHunk
  if (!apply) return

  activeHunkAction.value = { hunkId: hunk.id, mode }
  try {
    await Promise.resolve(apply(hunk.patch))
  } finally {
    const active = activeHunkAction.value
    if (active && active.hunkId === hunk.id && active.mode === mode) {
      activeHunkAction.value = null
    }
  }
}

async function handleEditorHunkAction(payload: { id: string; kind: HunkActionMode }) {
  const hunkId = String(payload?.id || '').trim()
  if (!hunkId) return
  const hunk = hunkById.value.get(hunkId)
  if (!hunk) return

  if (payload.kind === 'discard') {
    const confirmed = await confirmAction(String(t('git.ui.diffViewer.confirmDiscardHunk')))
    if (!confirmed) return
  }

  void runHunkAction(hunk, payload.kind)
}

async function load(opts: {
  directory: string
  path: string
  staged: boolean
  commit?: string
  parentCommit?: string
  signal: AbortSignal
  seq: number
  modelScope: string
}) {
  staleDiffText.value = diffText.value
  staleDiffMeta.value = diffMeta.value
  staleOriginal.value = original.value
  staleModified.value = modified.value

  loading.value = true
  error.value = null

  try {
    if (opts.commit) {
      const parentRequest = opts.parentCommit
        ? apiJson<GitCommitFileContentResponse>(
            `/api/v1/workbench/git/commit-file-content?directory=${encodeURIComponent(opts.directory)}&commit=${encodeURIComponent(opts.parentCommit)}&path=${encodeURIComponent(opts.path)}`,
            { signal: opts.signal },
          )
        : Promise.resolve<GitCommitFileContentResponse>({
            content: '',
            exists: false,
            binary: false,
            truncated: false,
          })

      const [diffResponse, originalResponse, modifiedResponse] = await Promise.all([
        apiJson<GitCommitDiffResponse>(
          `/api/v1/workbench/git/commit-file-diff?directory=${encodeURIComponent(opts.directory)}&commit=${encodeURIComponent(opts.commit)}&path=${encodeURIComponent(opts.path)}&contextLines=3`,
          { signal: opts.signal },
        ),
        parentRequest,
        apiJson<GitCommitFileContentResponse>(
          `/api/v1/workbench/git/commit-file-content?directory=${encodeURIComponent(opts.directory)}&commit=${encodeURIComponent(opts.commit)}&path=${encodeURIComponent(opts.path)}`,
          { signal: opts.signal },
        ),
      ])

      if (opts.signal.aborted || opts.seq !== loadSeq) return

      if (originalResponse?.binary || modifiedResponse?.binary) {
        error.value = t('files.timeline.errors.binaryUnavailable')
        diffText.value = ''
        diffMeta.value = null
        original.value = ''
        modified.value = ''
        return
      }

      diffText.value = diffResponse?.diff || ''
      diffMeta.value = null
      original.value = originalResponse?.exists ? originalResponse.content || '' : ''
      modified.value = modifiedResponse?.exists ? modifiedResponse.content || '' : ''
      loadedModelScope.value = opts.modelScope
      loadedLanguagePath.value = opts.path
      return
    }

    const [diffResponse, fileResponse] = await Promise.all([
      apiJson<GitDiffResponse>(
        `/api/v1/workbench/git/diff?directory=${encodeURIComponent(opts.directory)}&path=${encodeURIComponent(opts.path)}&staged=${opts.staged ? 'true' : 'false'}&contextLines=3&includeMeta=true`,
        { signal: opts.signal },
      ),
      apiJson<{ original: string; modified: string }>(
        `/api/v1/workbench/git/file-diff?directory=${encodeURIComponent(opts.directory)}&path=${encodeURIComponent(opts.path)}&staged=${opts.staged ? 'true' : 'false'}`,
        { signal: opts.signal },
      ),
    ])

    if (opts.signal.aborted || opts.seq !== loadSeq) return

    diffText.value = diffResponse?.diff || ''
    diffMeta.value = diffResponse?.meta && typeof diffResponse.meta === 'object' ? diffResponse.meta : null
    original.value = fileResponse?.original || ''
    modified.value = fileResponse?.modified || ''
    loadedModelScope.value = opts.modelScope
    loadedLanguagePath.value = opts.path
  } catch (e) {
    if (opts.signal.aborted || opts.seq !== loadSeq) return

    error.value = e instanceof Error ? e.message : String(e)
    throw e
  } finally {
    if (opts.signal.aborted || opts.seq !== loadSeq) return
    loading.value = false
  }
}

function openFile() {
  const path = normalizedPath.value
  if (!path || !props.onOpenFile) return
  props.onOpenFile(path)
}

function revealFile() {
  const path = normalizedPath.value
  if (!path || !props.onRevealFile) return
  props.onRevealFile(path)
}

async function readDiff() {
  if (!visible.value) return
  const directory = (props.directory || '').trim()
  const path = normalizedPath.value
  if (!directory || !path) return

  const ac = new AbortController()
  activeAbort = ac
  const seq = ++loadSeq

  const timeout = setTimeout(() => ac.abort(), 30_000)
  try {
    await load({
      directory,
      path,
      staged: Boolean(props.staged),
      commit: (props.commit || '').trim(),
      parentCommit: (props.parentCommit || '').trim(),
      signal: ac.signal,
      seq,
      modelScope: requestedModelScope.value,
    })
  } finally {
    clearTimeout(timeout)
    if (activeAbort === ac) {
      activeAbort = null
      loading.value = false
      if (ac.signal.aborted && seq === loadSeq && visible.value) {
        error.value = 'Diff request timed out'
        throw new Error(error.value)
      }
    }
  }
}

const makeRefreshQueue = () =>
  createRevalidator(readDiff, {
    intervalMs: 750,
    retryMs: 5000,
    enabled: () => visible.value && Boolean(normalizedDirectory.value && normalizedPath.value),
  })
let refreshQueue = makeRefreshQueue()
function refresh() {
  refreshQueue.invalidate(0)
  return refreshQueue.refresh().catch(() => {})
}

defineExpose({ refresh })

watch(
  requestedModelScope,
  () => {
    const directory = normalizedDirectory.value
    const path = normalizedPath.value

    refreshQueue.dispose()
    activeAbort?.abort()
    activeAbort = null
    loadSeq += 1
    loading.value = false
    refreshQueue = makeRefreshQueue()
    if (!directory || !path) {
      resetState()
      return
    }

    void refresh()
  },
  { immediate: true },
)
const visibility = () => {
  if (visible.value) {
    refreshQueue.resume()
  } else {
    refreshQueue.pause()
    const interrupted = Boolean(activeAbort)
    activeAbort?.abort()
    activeAbort = null
    loadSeq++
    loading.value = false
    if (interrupted) refreshQueue.invalidate(0)
  }
}
watch(visible, visibility, { flush: 'sync' })
onBeforeUnmount(() => {
  refreshQueue.dispose()
  activeAbort?.abort()
  loadSeq++
})
</script>

<template>
  <div class="git-editor-diff" :aria-busy="loading ? 'true' : 'false'">
    <div v-if="error" class="error">{{ error }}</div>

    <div v-if="isImageDiff" class="images">
      <div class="image-panel">
        <div class="image-title">{{ leftLabel }}</div>
        <button
          v-if="displayOriginal"
          type="button"
          class="preview-button"
          :aria-label="`${t('common.open')}: ${leftLabel}`"
          @click="openImageDiffPreview('left')"
        >
          <img :src="displayOriginal" class="preview cursor-zoom-in" alt="" aria-hidden="true" />
        </button>
        <div v-else class="hint">{{ t('git.ui.diffViewer.noOriginal') }}</div>
      </div>
      <div class="image-panel">
        <div class="image-title">{{ rightLabel }}</div>
        <button
          v-if="displayModified"
          type="button"
          class="preview-button"
          :aria-label="`${t('common.open')}: ${rightLabel}`"
          @click="openImageDiffPreview('right')"
        >
          <img :src="displayModified" class="preview cursor-zoom-in" alt="" aria-hidden="true" />
        </button>
        <div v-else class="hint">{{ t('git.ui.diffViewer.noModified') }}</div>
      </div>
    </div>

    <div v-else class="editor-shell">
      <div class="toolbar">
        <div v-if="path" class="path">{{ path }}</div>
        <div class="toolbar-actions">
          <IconButton
            variant="outline"
            size="sm"
            class="h-7 w-7 transition-colors"
            :class="
              wrapLines
                ? 'bg-secondary/70 text-foreground shadow-inner'
                : 'text-muted-foreground hover:bg-secondary/40 hover:text-foreground'
            "
            :title="wrapLines ? t('git.ui.diffViewer.wrap.disable') : t('git.ui.diffViewer.wrap.enable')"
            :aria-label="wrapLines ? t('git.ui.diffViewer.wrap.disable') : t('git.ui.diffViewer.wrap.enable')"
            :aria-pressed="wrapLines"
            @click="wrapLines = !wrapLines"
          >
            <RiTextWrap class="h-4 w-4" />
          </IconButton>
          <IconButton
            v-if="canOpenFile"
            variant="outline"
            size="sm"
            class="h-7 w-7 text-muted-foreground hover:bg-secondary/40 hover:text-foreground"
            :tooltip="t('git.ui.diffViewer.actions.openFile')"
            :aria-label="t('git.ui.diffViewer.actions.openFile')"
            @click="openFile"
          >
            <RiFileTextLine class="h-4 w-4" />
          </IconButton>
          <IconButton
            v-if="canRevealFile"
            variant="outline"
            size="sm"
            class="h-7 w-7 text-muted-foreground hover:bg-secondary/40 hover:text-foreground"
            :tooltip="t('git.ui.diffViewer.actions.revealInFiles')"
            :aria-label="t('git.ui.diffViewer.actions.revealInFiles')"
            @click="revealFile"
          >
            <RiSearchLine class="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      <div class="editor-container">
        <MonacoDiffEditor
          v-if="loadedModelScope"
          :original-value="displayOriginal"
          :modified-value="displayModified"
          :path="modifiedModelPath"
          :original-path="originalModelPath"
          :language-path="loadedLanguagePath"
          :initial-top-line="firstChangedLine"
          :use-files-theme="true"
          :wrap="wrapLines"
          :read-only="true"
          :hunk-actions="editorHunkActions"
          :hunk-actions-enabled="hasAnyHunkAction && editorHunkActions.length > 0"
          :hunk-actions-busy="loading || isAnyActionBusy"
          :active-hunk-action-id="activeHunkAction?.hunkId || null"
          :active-hunk-action-kind="activeHunkAction?.mode || null"
          @hunk-action="handleEditorHunkAction"
        />

        <div v-if="loading" class="loading-overlay" aria-hidden="true">
          <RiLoader4Line class="h-4 w-4 animate-spin" />
          <span>{{ t('git.ui.diffViewer.loading') }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.git-editor-diff {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.editor-shell {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
}

.toolbar {
  align-items: center;
  display: flex;
  gap: 8px;
  justify-content: space-between;
  min-height: 32px;
  padding: 0 8px;
}

.path {
  color: oklch(var(--muted-foreground));
  flex: 1;
  font-family: var(--font-mono);
  font-size: 11px;
  min-width: 0;
  overflow: hidden;
  padding-left: 2px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toolbar-actions {
  align-items: center;
  display: flex;
  gap: 6px;
}

.editor-container {
  border: 1px solid oklch(var(--border) / 0.6);
  border-radius: 10px;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  position: relative;
}

.loading-overlay {
  align-items: center;
  backdrop-filter: blur(1px);
  background: oklch(var(--background) / 0.52);
  color: oklch(var(--muted-foreground));
  display: inline-flex;
  font-size: 11px;
  gap: 6px;
  padding: 6px 10px;
  pointer-events: none;
  position: absolute;
  right: 10px;
  top: 10px;
}

.error {
  background: rgba(255, 120, 120, 0.08);
  border: 1px solid rgba(255, 120, 120, 0.26);
  border-radius: 10px;
  color: oklch(var(--destructive));
  font-size: 12px;
  padding: 10px 12px;
}

.images {
  display: grid;
  gap: 12px;
  grid-template-columns: 1fr 1fr;
  height: 100%;
  min-height: 0;
  padding: 12px;
}

.image-panel {
  align-content: start;
  background: oklch(var(--muted) / 0.15);
  border: 1px solid oklch(var(--border) / 0.6);
  border-radius: 10px;
  display: grid;
  gap: 8px;
  min-height: 0;
  padding: 10px;
}

.image-title {
  color: oklch(var(--muted-foreground));
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.preview {
  background: oklch(var(--background));
  border: 1px solid oklch(var(--border) / 0.6);
  border-radius: 8px;
  max-height: calc(100dvh - 260px);
  max-width: 100%;
  object-fit: contain;
}

.preview-button {
  background: transparent;
  border: 0;
  border-radius: 8px;
  justify-self: start;
  max-width: 100%;
  padding: 0;
}

.preview-button:focus-visible {
  outline: 2px solid oklch(var(--ring));
  outline-offset: 2px;
}

.hint {
  color: oklch(var(--muted-foreground));
  font-size: 12px;
}

@media (max-width: 920px) {
  .images {
    grid-template-columns: 1fr;
  }
}
</style>
