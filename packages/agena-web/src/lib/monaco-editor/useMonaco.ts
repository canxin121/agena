// Vendored from @guolao/vue-monaco-editor@1.6.0 (MIT).
import loader from '@monaco-editor/loader'
import { onMounted, ref, shallowRef, watch } from 'vue'

import type { MonacoEditor, Nullable } from './types'

type MonacoLoaderPromise = ReturnType<(typeof loader)['init']>

export function useMonaco(enabled: () => boolean = () => true) {
  const monacoRef = shallowRef<Nullable<MonacoEditor>>(loader.__getMonacoInstance() as Nullable<MonacoEditor>)
  const isLoadFailed = ref(false)
  let promise: MonacoLoaderPromise | undefined
  let disposed = false

  const initialize = () => {
    if (disposed || !enabled() || monacoRef.value || promise) return

    promise = loader.init()
    promise
      .then((monacoInstance) => {
        if (disposed) return
        monacoRef.value = monacoInstance as MonacoEditor
      })
      .catch((error) => {
        if (disposed) return
        if ((error as { type?: string } | undefined)?.type !== 'cancelation') {
          isLoadFailed.value = true
          console.error('Monaco initialization error:', error)
        }
      })
  }
  onMounted(initialize)
  watch(enabled, initialize)

  const unload = () => {
    disposed = true
    promise?.cancel()
  }

  return {
    monacoRef,
    unload,
    isLoadFailed,
  }
}

export { loader }
