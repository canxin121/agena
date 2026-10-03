import { loader } from './monaco-editor'

let monacoSetup: Promise<void> | null = null

export async function ensureMonacoReady() {
  if (monacoSetup) return monacoSetup
  monacoSetup = (async () => {
    const monaco = await import('monaco-editor')
    const [
      { default: editorWorker },
      { default: jsonWorker },
      { default: cssWorker },
      { default: htmlWorker },
      { default: tsWorker },
    ] = await Promise.all([
      import('monaco-editor/esm/vs/editor/editor.worker?worker'),
      import('monaco-editor/esm/vs/language/json/json.worker?worker'),
      import('monaco-editor/esm/vs/language/css/css.worker?worker'),
      import('monaco-editor/esm/vs/language/html/html.worker?worker'),
      import('monaco-editor/esm/vs/language/typescript/ts.worker?worker'),
    ])

    if (typeof self !== 'undefined') {
      const globalScope = self as typeof globalThis & {
        MonacoEnvironment?: { getWorker: (id: string, label: string) => Worker }
      }
      globalScope.MonacoEnvironment = {
        getWorker(_id, label) {
          if (label === 'json') return new jsonWorker()
          if (label === 'css' || label === 'scss' || label === 'less') return new cssWorker()
          if (label === 'html' || label === 'handlebars' || label === 'razor') return new htmlWorker()
          if (label === 'typescript' || label === 'javascript') return new tsWorker()
          return new editorWorker()
        },
      }
    }

    loader.config({ monaco })
  })()

  try {
    await monacoSetup
  } catch (error) {
    monacoSetup = null
    throw error
  }
}
