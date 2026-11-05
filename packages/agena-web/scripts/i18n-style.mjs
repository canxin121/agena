/**
 * Localized labels should not repeat their English translation in brackets.
 * Stable protocol IDs, keyboard shortcuts and command names belong in a
 * separate field (or unparenthesized explanatory text), not a bilingual gloss.
 */
const chineseEnglishGloss = /[\p{Script=Han}]\s*[（(]\s*[A-Za-z][^）)]*[）)]/u

export function findChineseEnglishGloss(value) {
  return String(value || '').match(chineseEnglishGloss)?.[0] || null
}
