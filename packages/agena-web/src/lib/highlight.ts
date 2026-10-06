import hljs from 'highlight.js/lib/core'

import bash from 'highlight.js/lib/languages/bash'
import css from 'highlight.js/lib/languages/css'
import diff from 'highlight.js/lib/languages/diff'
import go from 'highlight.js/lib/languages/go'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import markdown from 'highlight.js/lib/languages/markdown'
import plaintext from 'highlight.js/lib/languages/plaintext'
import python from 'highlight.js/lib/languages/python'
import rust from 'highlight.js/lib/languages/rust'
import scss from 'highlight.js/lib/languages/scss'
import typescript from 'highlight.js/lib/languages/typescript'
import xml from 'highlight.js/lib/languages/xml'

// Register common languages once. This module is imported in multiple renderers
// (markdown/code/diff) and should stay lightweight.
hljs.registerLanguage('bash', bash)
hljs.registerLanguage('css', css)
hljs.registerLanguage('diff', diff)
hljs.registerLanguage('go', go)
hljs.registerLanguage('javascript', javascript)
hljs.registerLanguage('json', json)
hljs.registerLanguage('markdown', markdown)
hljs.registerLanguage('plaintext', plaintext)
hljs.registerLanguage('python', python)
hljs.registerLanguage('rust', rust)
hljs.registerLanguage('scss', scss)
hljs.registerLanguage('typescript', typescript)
hljs.registerLanguage('xml', xml)

export { hljs }
const highlightedCache = new Map<string, string>()
let cacheCharacters = 0
const CACHE_CHARACTERS = 512 * 1024

function escapeHtml(input: string): string {
  return input
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;')
}

export function highlightCodeToHtml(code: string, lang?: string): string {
  const value = String(code ?? '')
  if (!value) return ''

  const requested = typeof lang === 'string' ? lang.trim().toLowerCase() : ''
  if (requested === 'text' || requested === 'plain' || requested === 'plaintext') {
    return escapeHtml(value)
  }
  // Auto detection runs every registered grammar; large payloads and very
  // long lines must never monopolize the UI thread. The text remains intact.
  const explicitLanguage = requested && hljs.getLanguage(requested)
  if (value.length > (explicitLanguage ? 16_384 : 2048)) return escapeHtml(value)
  let lineStart = 0
  for (let index = 0; index < value.length; index++) {
    if (value[index] === '\n') lineStart = index + 1
    else if (index - lineStart >= 4096) return escapeHtml(value)
  }
  const key = `${requested}\0${value}`
  const cached = highlightedCache.get(key)
  if (cached !== undefined) {
    highlightedCache.delete(key)
    highlightedCache.set(key, cached)
    return cached
  }
  let result: string
  try {
    if (explicitLanguage) {
      result = hljs.highlight(value, { language: requested, ignoreIllegals: true }).value
    } else {
      result = hljs.highlightAuto(value).value
    }
  } catch {
    result = escapeHtml(value)
  }
  const size = key.length + result.length
  if (size <= CACHE_CHARACTERS) {
    while (cacheCharacters + size > CACHE_CHARACTERS || highlightedCache.size >= 256) {
      const oldest = highlightedCache.keys().next().value!
      cacheCharacters -= oldest.length + highlightedCache.get(oldest)!.length
      highlightedCache.delete(oldest)
    }
    highlightedCache.set(key, result)
    cacheCharacters += size
  }
  return result
}
