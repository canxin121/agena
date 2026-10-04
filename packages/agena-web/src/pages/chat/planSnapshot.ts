import type { JsonValue } from '@/types/json'

export type PlanSnapshot = {
  title: string
  phase: string
  progress: string
  currentStep: string
  completed: number
  total: number
  autorun: boolean | null
}

function record(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}

/** Read the canonical plan payload, independently of paged chat history. */
export function readPlanSnapshot(response: Record<string, JsonValue>): PlanSnapshot | null {
  const payload = record(response.payload)
  const plan = record(payload.plan)
  if (!Object.keys(plan).length) return null
  const title = String(plan.title || plan.objective || '').trim()
  const steps = Array.isArray(plan.steps) ? plan.steps.map(record) : []
  const completed = steps.filter((step) => step.status === 'completed' || step.status === 'skipped').length
  const phase = typeof plan.phase === 'string' ? plan.phase : ''
  const symbol =
    ({ completed: '✓', blocked: '⚠', cancelled: '✕', planning: '⏳' } as Record<string, string>)[phase] || '▶'
  const autorun = typeof plan.autorun === 'boolean' ? plan.autorun : null
  const current = record(payload.current_step)
  return {
    title,
    phase,
    progress: [symbol, steps.length ? `${completed}/${steps.length}` : '', autorun ? '↻' : '']
      .filter(Boolean)
      .join(' '),
    currentStep: phase === 'completed' || phase === 'cancelled' ? '' : String(current.title || '').trim(),
    completed,
    total: steps.length,
    autorun,
  }
}

/** Revision is transport metadata, not part of the human-facing plan document. */
export function planDocument(response: Record<string, JsonValue>): string {
  return typeof response.output_text === 'string' ? response.output_text.replace(/^Revision:[^\n]*\n/, '').trim() : ''
}
