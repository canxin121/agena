import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'
import { remixiconTreeShaking } from '../scripts/remixiconTreeShaking'

test(
  'a real icon build keeps imported components and discards unused icon constructors',
  { timeout: 20_000 },
  async () => {
    const result = await build({
      root: fileURLToPath(new URL('..', import.meta.url)),
      configFile: false,
      logLevel: 'silent',
      plugins: [remixiconTreeShaking()],
      build: {
        write: false,
        minify: false,
        sourcemap: true,
        lib: { entry: fileURLToPath(new URL('./fixtures/iconTreeShakingEntry.ts', import.meta.url)), formats: ['es'] },
        rolldownOptions: { external: ['vue'] },
      },
    })
    const bundles = Array.isArray(result) ? result : [result]
    const chunks = bundles
      .flatMap((bundle) => ('output' in bundle ? bundle.output : []))
      .filter((chunk) => chunk.type === 'chunk')
    const code = chunks.map((chunk) => chunk.code).join('\n')
    assert.ok(code.includes('RiAddLine'))
    assert.ok(code.includes('RiCloseLine'))
    assert.equal(code.includes('Ri24HoursFill'), false)
    assert.equal((code.match(/viewBox:/g) || []).length, 2)
    assert.ok(code.length < 10_000, 'two icons must not retain the complete multi-megabyte library')
    assert.ok(
      chunks.every((chunk) => chunk.map?.sourcesContent?.some((source) => source?.includes('RiAddLine'))),
      'debug sourcemaps must retain the icon source',
    )
  },
)
