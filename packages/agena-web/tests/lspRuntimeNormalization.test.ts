import assert from 'node:assert/strict'
import test from 'node:test'

import { normalizeLspRuntimeList } from '../src/lib/lspRuntime'

test('normalizeLspRuntimeList projects the canonical runtime operator LSP shape', () => {
  const items = normalizeLspRuntimeList({
    operator: {
      lsp: {
        servers: [
          {
            name: 'rust-analyzer',
            command: 'rust-analyzer',
            file_extensions: ['rs'],
            root_markers: ['Cargo.toml'],
          },
          {
            name: 'typescript-language-server',
            command: 'typescript-language-server --stdio',
            file_extensions: ['ts', 'tsx'],
            root_markers: ['package.json'],
          },
        ],
      },
    },
  })

  assert.deepEqual(items, [
    {
      id: 'rust-analyzer',
      name: 'rust-analyzer',
      status: 'configured',
      transport: 'rust-analyzer',
      fileExtensions: ['rs'],
      rootMarkers: ['Cargo.toml'],
    },
    {
      id: 'typescript-language-server',
      name: 'typescript-language-server',
      status: 'configured',
      transport: 'typescript-language-server --stdio',
      fileExtensions: ['ts', 'tsx'],
      rootMarkers: ['package.json'],
    },
  ])
})

test('normalizeLspRuntimeList rejects alternate list and field spellings', () => {
  assert.deepEqual(normalizeLspRuntimeList({ items: [{ id: 'rust-analyzer' }] } as never), [])
  assert.deepEqual(
    normalizeLspRuntimeList({
      operator: {
        lsp: {
          servers: [{ id: 'rust-analyzer', transport: 'stdio' }],
        },
      },
    }),
    [],
  )
})
