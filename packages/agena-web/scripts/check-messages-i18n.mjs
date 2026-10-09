// Guard rails for the general UI message catalogs (src/i18n/messages/*).
//
// - Every statically referenced `t()` / `$t()` / `i18n.global.t()` key must
//   exist in en-US (vue-i18n would otherwise render the raw key).
// - en-US and zh-CN carry the same leaf keys with the same placeholders;
//   the other locales are intentionally partial and fall back to en-US.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

import enUS from '../src/i18n/messages/en-US.ts'
import zhCN from '../src/i18n/messages/zh-CN.ts'

const ROOT = new URL('../src', import.meta.url).pathname
const errors = []

function flatten(value, prefix = '', output = {}) {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) flatten(child, prefix ? `${prefix}.${key}` : key, output)
  } else if (typeof value === 'string') output[prefix] = value
  return output
}

const en = flatten(enUS)
const zh = flatten(zhCN)
const enKeys = new Set(Object.keys(en))
const zhKeys = new Set(Object.keys(zh))

const missingInZh = [...enKeys].filter((key) => !zhKeys.has(key))
if (missingInZh.length) errors.push(`zh-CN is missing en-US keys: ${missingInZh.slice(0, 20).join(' | ')}`)
const extraInZh = [...zhKeys].filter((key) => !enKeys.has(key))
if (extraInZh.length) errors.push(`zh-CN has keys absent from en-US: ${extraInZh.slice(0, 20).join(' | ')}`)

function placeholders(value) {
  return [...String(value).matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]).sort()
}
for (const key of enKeys) {
  if (!String(en[key]).trim()) errors.push(`en-US value is empty: ${key}`)
  if (!zhKeys.has(key)) continue
  if (!String(zh[key]).trim()) errors.push(`zh-CN value is empty: ${key}`)
  const expected = JSON.stringify(placeholders(en[key]))
  const actual = JSON.stringify(placeholders(zh[key]))
  if (expected !== actual) errors.push(`zh-CN placeholder mismatch for ${key}: expected=${expected} actual=${actual}`)
}

const files = []
function walk(dir) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) walk(path)
    else if (/\.(?:ts|vue)$/.test(entry)) files.push(path)
  }
}
walk(ROOT)

// `te()` / `$te()` only probes for a key, so they are not required to exist.
const staticKeyPatterns = [
  /(?:^|[^\w$.'"])\$t\(\s*(['"])((?:\\.|(?!\1).)*)\1/g,
  /(?:^|[^\w$.'"])(?:t|te)?\.?t\(\s*(['"])((?:\\.|(?!\1).)*)\1/g,
  /i18n\.global\.t\(\s*(['"])((?:\\.|(?!\1).)*)\1/g,
]
const referenced = new Map()
for (const file of files) {
  const source = readFileSync(file, 'utf8')
  for (const pattern of staticKeyPatterns) {
    for (const match of source.matchAll(pattern)) {
      const raw = match[2]
      if (!raw || !/[A-Za-z]/.test(raw) || !raw.includes('.')) continue
      if (!referenced.has(raw)) referenced.set(raw, relative(ROOT, file))
    }
  }
}
for (const [key, file] of [...referenced].sort()) {
  if (!enKeys.has(key)) errors.push(`missing en-US key for ${key} (referenced in ${file})`)
}

if (errors.length) {
  console.error('Message i18n check failed:\n')
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}
console.log(
  `Message i18n check passed: ${referenced.size} referenced keys, ${enKeys.size} en-US keys, ${zhKeys.size} zh-CN keys`,
)
