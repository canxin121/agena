import { onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { loadBrowserResource } from './authenticatedResources'
import { isAuthenticatedResourceUrl } from './resourceUrls'
import { readServerAuthTokenVersion, subscribeServerAuthTokenVersion } from './serverAuthToken'

/** Own each preview's object URL for exactly as long as its source is mounted.
 * Replacing a part or switching sessions cancels late loads and releases bytes. */
export function useAuthenticatedResource(source: MaybeRefOrGetter<string>) {
  const url = ref('')
  const error = ref('')
  const loading = ref(false)
  const authVersion = ref(readServerAuthTokenVersion())
  let cancelCurrent = () => {}
  const unsubscribe = subscribeServerAuthTokenVersion((version) => {
    if (isAuthenticatedResourceUrl(toValue(source))) {
      cancelCurrent()
      // Clear the old pixels synchronously; a new token may reload the same
      // source, while logout can unmount the protected page before that read.
      url.value = ''
      error.value = ''
      loading.value = false
    }
    authVersion.value = version
  })
  onScopeDispose(() => {
    unsubscribe()
    cancelCurrent()
  })
  watch(
    () => [toValue(source), isAuthenticatedResourceUrl(toValue(source)) ? authVersion.value : null] as const,
    async ([sourceUrl], _, onCleanup) => {
      const controller = new AbortController()
      let release = () => {}
      cancelCurrent = () => {
        controller.abort()
        release()
      }
      onCleanup(cancelCurrent)
      url.value = ''
      error.value = ''
      loading.value = Boolean(sourceUrl)
      try {
        const resource = await loadBrowserResource(sourceUrl, controller.signal)
        if (controller.signal.aborted) {
          resource.release()
          return
        }
        release = resource.release
        url.value = resource.url
      } catch (cause) {
        if (!controller.signal.aborted) error.value = cause instanceof Error ? cause.message : String(cause)
      } finally {
        if (!controller.signal.aborted) loading.value = false
      }
    },
    { immediate: true },
  )
  return { url, error, loading }
}
