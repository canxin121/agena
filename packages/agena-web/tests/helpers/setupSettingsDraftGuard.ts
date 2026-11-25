import { readFileSync } from 'node:fs'
import { transformSync } from 'esbuild'
import { readWindowIdFromQuery } from '../../src/app/windowScope'

const code = transformSync(
  readFileSync(new URL('../../src/components/settings/useSettingsDraftGuard.ts', import.meta.url), 'utf8'),
  { loader: 'ts', format: 'cjs' },
).code

/** Run the shared guard with the editor fixture's router, dialog and lifecycle hooks. */
export function setupSettingsDraftGuard(modules: Record<string, unknown>) {
  modules['@/app/windowScope'] ??= { readWindowIdFromQuery }
  modules['@/app/workspace/workspacePaneContext'] ??= { useWorkspacePaneContext: () => null }
  const module = { exports: {} }
  new Function('require', 'module', 'exports', code)(
    (id: string) => {
      if (!(id in modules)) throw new Error('Unexpected settings guard import: ' + id)
      return modules[id]
    },
    module,
    module.exports,
  )
  modules['./useSettingsDraftGuard'] = module.exports
}
