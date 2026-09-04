import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const inputSource = readFileSync(new URL('../src/components/ui/Input.vue', import.meta.url), 'utf8')
const schemaSource = readFileSync(new URL('../src/components/settings/plugins/JsonSchemaField.vue', import.meta.url), 'utf8')
const contractSource = readFileSync(
  new URL('../src/components/settings/PluginContractEditor.vue', import.meta.url),
  'utf8',
)

test('shared Input can shrink inside narrow flex and grid layouts', () => {
  assert.match(inputSource, /h-9 w-full min-w-0/)
})

test('dynamic settings scalar controls inherit their visible schema title as an accessible name', () => {
  const schemaLabels = schemaSource.match(/:aria-label="title"/g) || []
  assert.ok(schemaLabels.length >= 4)

  const contractLabels = contractSource.match(/:aria-label="node\.title \|\| node\.id"/g) || []
  assert.ok(contractLabels.length >= 5)
  assert.match(contractSource, /:aria-label="\$st\('New entry name'\)"/)
})
