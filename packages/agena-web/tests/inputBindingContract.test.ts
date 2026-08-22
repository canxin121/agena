import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

function vueFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) return vueFiles(path)
    return entry.isFile() && path.endsWith('.vue') ? [path] : []
  })
}

test('all custom Input usages use the component model-value contract', () => {
  const sourceRoot = resolve(import.meta.dir, '../src')
  const inputTags = vueFiles(sourceRoot).flatMap((file) => {
    const source = readFileSync(file, 'utf8')
    return [...source.matchAll(/<Input\b[\s\S]*?\/>/g)].map((match) => ({ file, tag: match[0] }))
  })

  assert.ok(inputTags.length > 0)
  for (const { file, tag } of inputTags) {
    assert.equal(tag.includes(':value='), false, `${file} passes native :value to Input`)
    assert.equal(tag.includes('@input='), false, `${file} listens to native @input on Input`)
    assert.match(tag, /(?:v-model|:model-value=|:modelValue=)/, `${file} does not control Input through modelValue`)
  }
})
