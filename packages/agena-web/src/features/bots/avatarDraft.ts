import { ref } from 'vue'
import { BotAvatarError, normalizeBotAvatar, type BotAvatarErrorCode } from './avatar'

// The preview and the optional update field have different meanings: an
// untouched avatar must be omitted from a rename, while null removes it.
export function useBotAvatarDraft(normalize = normalizeBotAvatar) {
  const avatar = ref<string | null>(null)
  const change = ref<string | null | undefined>(undefined)
  const loading = ref(false)
  const error = ref<BotAvatarErrorCode | null>(null)
  let original: string | null = null
  let generation = 0
  let request: AbortController | null = null

  function cancel() {
    generation++
    request?.abort()
    request = null
    loading.value = false
  }

  function reset(value?: string | null) {
    cancel()
    original = value || null
    avatar.value = original
    change.value = undefined
    error.value = null
  }

  function remove() {
    cancel()
    avatar.value = null
    change.value = original === null ? undefined : null
    error.value = null
  }

  async function upload(file: File) {
    cancel()
    const current = generation
    const abort = new AbortController()
    request = abort
    loading.value = true
    error.value = null
    try {
      const value = await normalize(file, abort.signal)
      if (current !== generation || abort.signal.aborted) return
      avatar.value = value
      change.value = value === original ? undefined : value
    } catch (cause) {
      if (current !== generation || abort.signal.aborted) return
      error.value = cause instanceof BotAvatarError ? cause.code : 'read'
    } finally {
      if (current === generation) {
        request = null
        loading.value = false
      }
    }
  }

  return { avatar, change, loading, error, reset, remove, upload, cancel }
}
