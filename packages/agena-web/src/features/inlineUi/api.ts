import { apiJson } from '@/lib/api'
import { parseInlineDocument } from './model'
import { validInlineReference } from './references'

/** Always scoped to the selected session. No global cache can retain revoked
 * membership or accidentally lend content to a different conversation. */
export async function readInlineReference(sessionId: string, reference: string, signal?: AbortSignal) {
  if (!sessionId || !validInlineReference(reference)) throw new Error('Invalid inline reference')
  const value = await apiJson<unknown>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/inline-ui/${encodeURIComponent(reference)}`,
    { signal, cache: 'no-store' },
  )
  return parseInlineDocument(value)
}
