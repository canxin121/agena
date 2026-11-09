import { readSessionIdFromQuery } from './sessionQuery'

/** Recognize explicit app conversations before Markdown's filesystem handling. */
export function appConversationLink(href: string, origin: string): string | null {
  if (!href.startsWith('/') && !/^https?:\/\//i.test(href)) return null
  try {
    const url = new URL(href, origin)
    if (url.origin !== origin || url.pathname !== '/chat') return null
    if (!readSessionIdFromQuery(Object.fromEntries(url.searchParams))) return null
    return `${url.pathname}${url.search}${url.hash}`
  } catch {
    return null
  }
}
