import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('MCP status dialog reads the canonical Agena runtime projection', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/McpDialog.vue'), 'utf8')

  assert.ok(source.includes("apiJson<RuntimeStatus>('/api/v1/runtime')"))
  assert.ok(source.includes('normalizeMcpStatus(data)'))
  assert.ok(source.includes('runtime.value?.operator?.mcp?.server_count'))
  assert.ok(source.includes('runtime.value?.operator?.mcp?.tool_count'))
  assert.ok(source.includes('<RiRefreshLine'))
  assert.ok(!source.includes("'/api/mcp'"))
})
