import { ref, shallowRef } from 'vue'

import { connectSse } from '@/lib/sse'
import { apiUrl } from '@/lib/api'
import { readUiAuthTokenVersion } from '@/lib/uiAuthToken'

export function useGitWatchSse<TPayload>(opts: {
  buildUrl: (directory: string) => string
  onPayload: (payload: TPayload, prev: TPayload | null) => void
  onError?: () => void
}) {
  const watchSource = ref<ReturnType<typeof connectSse> | null>(null)
  const watchRefreshTimer = ref<number | null>(null)
  const watchLastPayload = shallowRef<TPayload | null>(null)
  let watchDirectory: string | null = null
  let watchIdentity = ''
  let generation = 0
  const previousPayloads = new Map<string, TPayload>()

  function stopWatch() {
    generation++
    if (watchIdentity && watchLastPayload.value) {
      previousPayloads.delete(watchIdentity)
      previousPayloads.set(watchIdentity, watchLastPayload.value as TPayload)
      if (previousPayloads.size > 16) previousPayloads.delete(previousPayloads.keys().next().value!)
    }
    if (watchRefreshTimer.value) {
      window.clearTimeout(watchRefreshTimer.value)
      watchRefreshTimer.value = null
    }
    if (watchSource.value) {
      watchSource.value.close()
      watchSource.value = null
    }
    watchLastPayload.value = null
    watchDirectory = null
    watchIdentity = ''
  }

  function startWatch(directory: string, identity = directory) {
    const key = `${readUiAuthTokenVersion()}:${apiUrl(opts.buildUrl(directory))}:${identity}`
    if (watchSource.value && watchDirectory === directory && watchIdentity === key) return
    stopWatch()
    watchDirectory = directory
    watchIdentity = key
    watchLastPayload.value = previousPayloads.get(key) ?? null
    const owner = generation
    const endpoint = opts.buildUrl(directory)
    const client = connectSse({
      endpoint,
      debugLabel: 'sse:git-watch',
      onEvent: (evt) => {
        if (owner !== generation) return
        if (String(evt?.type || '') !== 'git.watch.status') return
        const payload = (evt as unknown as { properties?: unknown }).properties as TPayload | undefined
        if (!payload) return
        const prev = watchLastPayload.value
        watchLastPayload.value = payload
        opts.onPayload(payload, prev)
      },
      onError: () => {
        opts.onError?.()
        // connectSse owns reconnects and preserves exponential backoff.
      },
    })
    watchSource.value = client
  }

  return {
    watchSource,
    watchLastPayload,
    watchRefreshTimer,
    startWatch,
    stopWatch,
  }
}
