import { ref, watch } from 'vue'
import { i18n } from '@/i18n'

import type { GitCompareResponse } from '@/types/git'
import type { JsonValue } from '@/types/json'

type QueryValue = string | number | boolean | null | undefined

type GitJson = <T = JsonValue>(
  endpoint: string,
  directory: string,
  query?: Record<string, QueryValue> | undefined,
  init?: RequestInit | undefined,
) => Promise<T>

type ToastKind = 'info' | 'success' | 'error'
type Toasts = { push: (kind: ToastKind, message: string, timeoutMs?: number) => void }

export function useGitCompareOps(opts: { repoRoot: { value: string | null }; gitJson: GitJson; toasts: Toasts }) {
  const { repoRoot, gitJson, toasts } = opts

  const compareOpen = ref(false)
  const compareBase = ref('')
  const compareHead = ref('')
  const comparePath = ref('')
  const compareDiff = ref('')
  const compareLoading = ref(false)
  const compareError = ref<string | null>(null)
  let compareSequence = 0
  watch(
    () => repoRoot.value,
    () => {
      compareSequence++
      compareLoading.value = false
      compareOpen.value = false
      compareDiff.value = ''
      compareError.value = null
    },
  )

  function openCompareDialog() {
    compareOpen.value = true
  }

  function closeCompareDialog() {
    compareOpen.value = false
  }

  function resetCompare() {
    compareDiff.value = ''
    compareError.value = null
  }

  function swapCompareRefs() {
    const nextBase = compareHead.value
    const nextHead = compareBase.value
    compareBase.value = nextBase
    compareHead.value = nextHead
  }

  async function runCompare(opts?: { base?: string; head?: string; path?: string }) {
    const dir = repoRoot.value
    if (!dir) return
    const base = (opts?.base ?? compareBase.value).trim()
    const head = (opts?.head ?? compareHead.value).trim()
    const path = (opts?.path ?? comparePath.value).trim()
    if (!base || !head) return
    compareBase.value = base
    compareHead.value = head
    comparePath.value = path
    const sequence = ++compareSequence
    compareLoading.value = true
    compareError.value = null
    try {
      const resp = await gitJson<GitCompareResponse>('compare', dir, {
        base,
        head,
        path: path || undefined,
        contextLines: 3,
      })
      if (sequence !== compareSequence || dir !== repoRoot.value) return
      compareDiff.value = resp?.diff || ''
      if (!compareDiff.value.trim()) {
        toasts.push('info', i18n.global.t('git.toasts.noDifferencesFound'))
      }
    } catch (err) {
      if (sequence !== compareSequence || dir !== repoRoot.value) return
      compareError.value = err instanceof Error ? err.message : String(err)
    } finally {
      if (sequence === compareSequence) compareLoading.value = false
    }
  }

  async function compareCommitWithParent(commitHash: string, parentHash: string, path?: string | null) {
    const head = (commitHash || '').trim()
    const base = (parentHash || '').trim()
    if (!head || !base) return
    compareOpen.value = true
    await runCompare({
      base,
      head,
      path: (path || '').trim(),
    })
  }

  return {
    compareOpen,
    compareBase,
    compareHead,
    comparePath,
    compareDiff,
    compareLoading,
    compareError,
    openCompareDialog,
    closeCompareDialog,
    resetCompare,
    swapCompareRefs,
    compareCommitWithParent,
    runCompare,
  }
}
