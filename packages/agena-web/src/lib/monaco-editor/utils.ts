// Vendored from @guolao/vue-monaco-editor@1.6.0 (MIT).
import type { MonacoEditor } from './types'

export function slotHelper<T>(slot: T | (() => T)) {
  return typeof slot === 'function' ? (slot as () => T)() : slot
}

export function isUndefined<T>(v: T | undefined): v is undefined {
  return v === undefined
}

type TextModel = ReturnType<MonacoEditor['editor']['createModel']>
const modelUsers = new WeakMap<TextModel, { count: number; owned: boolean }>()

/** Editors share named models, but only retain the model they are displaying.
 * A model created elsewhere is never disposed by this wrapper.
 */
export function acquireModel(monaco: MonacoEditor, value: string, language?: string, path?: string) {
  const existing = path ? getModel(monaco, path) : null
  const model = existing || createModel(monaco, value, language, path)
  let users = modelUsers.get(model)
  if (!users) {
    users = { count: 0, owned: !existing }
    modelUsers.set(model, users)
  }
  users.count++
  let released = false
  return {
    model,
    release() {
      if (released) return
      released = true
      if (--users.count === 0) {
        modelUsers.delete(model)
        if (users.owned && !model.isDisposed()) model.dispose()
      }
    },
  }
}

function getModel(monaco: MonacoEditor, path: string) {
  return monaco.editor.getModel(createModelUri(monaco, path))
}

function createModel(monaco: MonacoEditor, value: string, language?: string, path?: string) {
  return monaco.editor.createModel(value, language, path ? createModelUri(monaco, path) : undefined)
}

function createModelUri(monaco: MonacoEditor, path: string) {
  return monaco.Uri.parse(path)
}
