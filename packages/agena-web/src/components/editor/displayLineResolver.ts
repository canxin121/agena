/** Preserve the first mapped line >= the requested display line, including
 * sparse/non-monotonic maps, without scanning the entire file for every hunk. */
export function createDisplayLineResolver(
  lineCount: number,
  map: Array<number | null> | null,
  startLine: number | null,
) {
  const maxima: number[] = []
  const lines: number[] = []
  let lastMapped = 0
  if (map) {
    for (let index = 0; index < Math.min(map.length, lineCount); index++) {
      const value = Number(map[index])
      if (!Number.isFinite(value) || value <= 0) continue
      const mapped = Math.floor(value)
      lastMapped = index + 1
      if (!maxima.length || mapped > maxima[maxima.length - 1]!) {
        maxima.push(mapped)
        lines.push(lastMapped)
      }
    }
  }
  return (displayLine: number) => {
    const target = Number.isFinite(displayLine) && displayLine > 0 ? Math.floor(displayLine) : 1
    if (lastMapped) {
      let low = 0,
        high = maxima.length
      while (low < high) {
        const middle = (low + high) >>> 1
        if (maxima[middle]! < target) low = middle + 1
        else high = middle
      }
      return lines[low] ?? lastMapped
    }
    return Math.max(1, Math.min(lineCount, startLine === null ? target : target - startLine + 1))
  }
}
