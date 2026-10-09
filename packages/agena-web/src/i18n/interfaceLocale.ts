import { i18n } from '@/i18n'
import { normalizeAppLocale } from '@/i18n/locale'

/**
 * Append the active interface language to a request URL.
 *
 * Human headlines are stored English data: the server maps its known
 * vocabulary for the requested language instead of rewriting stored facts, so
 * every request that can return a headline carries the language. The parameter
 * merges into an existing query string rather than replacing it.
 */
export function withInterfaceLocale(url: string): string {
  const locale = normalizeAppLocale(i18n.global.locale.value)
  if (!locale) return url
  return `${url}${url.includes('?') ? '&' : '?'}locale=${encodeURIComponent(locale)}`
}
