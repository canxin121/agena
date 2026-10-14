import { nextTick } from 'vue'

/** Share a small task queue so expensive mounts/cleanup cannot run in the
 * input event's microtask flush, before the browser paints its response. */
const pending = new Set<() => void>()
let scheduled = false

function schedule() {
  if (scheduled || !pending.size) return
  scheduled = true
  let started = false
  const drain = async () => {
    if (started) return
    started = true
    clearTimeout(fallback)
    if (frame !== undefined) cancelAnimationFrame(frame)
    const start = performance.now()
    try {
      for (const task of pending) {
        pending.delete(task)
        task()
        // Count the resulting Vue mount/unmount, not just the cheap ref write.
        await nextTick()
        if (performance.now() - start >= 4) break
      }
    } finally {
      scheduled = false
      schedule()
    }
  }
  // rAF itself runs before paint. The following task gives the frame a chance
  // to appear first. The fallback also drains when the document is hidden.
  const fallback = setTimeout(drain, 100)
  const frame =
    typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => setTimeout(drain, 0)) : undefined
}

export function afterPaint(task: () => void): () => void {
  pending.add(task)
  schedule()
  return () => {
    pending.delete(task)
  }
}
