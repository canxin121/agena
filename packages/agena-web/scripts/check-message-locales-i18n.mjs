// Enforce complete user-visible message catalogs for every Web-supported locale.
// vue-i18n still falls back to en-US for genuinely unknown dynamic keys; a
// supported locale must not rely on that fallback for known catalog entries.
import enUS from '../src/i18n/messages/en-US.ts'
import zhCN from '../src/i18n/messages/zh-CN.ts'
import esES from '../src/i18n/messages/es-ES.ts'
import frFR from '../src/i18n/messages/fr-FR.ts'
import hiIN from '../src/i18n/messages/hi-IN.ts'
import arSA from '../src/i18n/messages/ar-SA.ts'
import ptBR from '../src/i18n/messages/pt-BR.ts'

const catalogs = {
  'en-US': enUS,
  'zh-CN': zhCN,
  'es-ES': esES,
  'fr-FR': frFR,
  'hi-IN': hiIN,
  'ar-SA': arSA,
  'pt-BR': ptBR,
}
const errors = []

function flatten(value, prefix = '', output = {}) {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      flatten(child, prefix ? `${prefix}.${key}` : key, output)
    }
  } else if (typeof value === 'string') {
    output[prefix] = value
  }
  return output
}

function placeholders(value) {
  return [...String(value).matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]).sort()
}

// Catch a supported locale that copied a complete English sentence instead of
// translating it. Short labels and identifiers (Git, MCP, true/false, hotkeys,
// model names, paths, etc.) are intentionally excluded from this heuristic.
const englishFunctionWords = /\b(the|and|to|for|with|your|you|are|is|will|can|cannot|please|this|that|from|when|while|after|before|enter|select|choose|open|close|save|delete|search|loading|failed|continue)\b/i
function looksLikeUntranslatedEnglish(value) {
  return String(value).length > 24 && englishFunctionWords.test(String(value))
}

const english = flatten(enUS)
const englishKeys = new Set(Object.keys(english))
for (const [locale, catalog] of Object.entries(catalogs)) {
  const messages = flatten(catalog)
  const keys = new Set(Object.keys(messages))
  const missing = [...englishKeys].filter((key) => !keys.has(key))
  const extra = [...keys].filter((key) => !englishKeys.has(key))
  if (missing.length) errors.push(`${locale} is missing ${missing.length} keys: ${missing.slice(0, 20).join(', ')}`)
  if (extra.length) errors.push(`${locale} has ${extra.length} unknown keys: ${extra.slice(0, 20).join(', ')}`)
  for (const key of englishKeys) {
    if (!keys.has(key)) continue
    if (!String(messages[key]).trim()) errors.push(`${locale} has an empty value for ${key}`)
    if (JSON.stringify(placeholders(english[key])) !== JSON.stringify(placeholders(messages[key]))) {
      errors.push(`${locale} has placeholder mismatch for ${key}`)
    }
    if (
      locale !== 'en-US' &&
      messages[key] === english[key] &&
      looksLikeUntranslatedEnglish(messages[key])
    ) {
      errors.push(`${locale} has untranslated English copy for ${key}`)
    }
  }
  console.log(`${locale}: ${keys.size}/${englishKeys.size} message keys`)
}

if (errors.length) {
  console.error('\nLocale catalog completeness check failed:\n')
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}
console.log(`Locale catalog completeness check passed: ${englishKeys.size} keys across ${Object.keys(catalogs).length} locales`)
