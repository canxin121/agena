import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

function extractFunctionSource(source: string, signature: string): string {
  const start = source.indexOf(signature)
  assert.ok(start >= 0, `missing function signature: ${signature}`)

  const blockStart = source.indexOf('{', start)
  assert.ok(blockStart >= 0, `missing block start for: ${signature}`)

  let depth = 0
  for (let idx = blockStart; idx < source.length; idx += 1) {
    const ch = source[idx]
    if (ch === '{') depth += 1
    else if (ch === '}') depth -= 1
    if (depth === 0) return source.slice(start, idx + 1)
  }

  throw new Error(`unterminated function block: ${signature}`)
}

test('closing the final workspace window keeps the empty shell on the chat sidebar', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/stores/ui.ts'), 'utf8')
  const closeWindow = extractFunctionSource(source, 'function closeWorkspaceWindow(windowId: string)')
  const closeAll = extractFunctionSource(source, 'function closeAllWorkspaceWindows(')

  assert.ok(closeWindow.includes('if (workspaceWindows.value.length <= 1)'))
  assert.ok(closeWindow.includes("activeMainTabFallback.value = 'chat'"))
  assert.ok(closeAll.includes("activeMainTabFallback.value = 'chat'"))
  assert.ok(
    source.includes(
      "const activeMainTabFallback = ref<MainTab>(workspaceWindows.value.length ? defaultMainTab : 'chat')",
    ),
  )
})

test('a late route notification cannot resurrect a workspace window that was just closed', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/stores/ui.ts'), 'utf8')

  assert.ok(source.includes('isRouteWindowRestoreSuppressed(routeWindowId)'))
  assert.ok(source.includes('if (routeWindowId && isRouteWindowRestoreSuppressed(routeWindowId))'))
  assert.ok(source.includes("rawRoutePath === '/'"))
})
