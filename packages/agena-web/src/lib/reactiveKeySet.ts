/** Update key dependencies only where membership changes. Reading has(key)
 * in a row subscribes that row to its key rather than the entire selection.
 */
export function syncKeySet(target: Set<string>, keys: Iterable<string>) {
  const next = new Set(keys)
  for (const key of target) if (!next.has(key)) target.delete(key)
  for (const key of next) if (!target.has(key)) target.add(key)
}
