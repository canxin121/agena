import { readActiveBackendBaseUrl } from '../../lib/backend'
import type { TaskForm } from './taskBoard'

const volatileDrafts = new Map<string, TaskForm>()

function storageKey(source: number | null): string {
  return `agena.task.draft.v1:${readActiveBackendBaseUrl()}:${source ?? 'none'}`
}
/** Session-scoped, bounded recovery only; never sync task text across backends. */
export function saveTaskDraft(source: number | null, form: TaskForm | null): void {
  if (form) volatileDrafts.set(storageKey(source), JSON.parse(JSON.stringify(form)) as TaskForm)
  else volatileDrafts.delete(storageKey(source))
  try {
    if (form) sessionStorage.setItem(storageKey(source), JSON.stringify(form))
    else sessionStorage.removeItem(storageKey(source))
  } catch {
    /* Private storage/quota failures must not stop an in-memory form. */
  }
}
export function loadTaskDraft(source: number | null): TaskForm | null {
  const cached = volatileDrafts.get(storageKey(source))
  if (cached) return JSON.parse(JSON.stringify(cached)) as TaskForm
  try {
    const raw = sessionStorage.getItem(storageKey(source))
    if (!raw || raw.length > 250000) return null
    const value = JSON.parse(raw) as TaskForm
    if (
      !['create', 'resume', 'cancel', 'accept'].includes(value.kind) ||
      typeof value.text !== 'string' ||
      typeof value.key !== 'string'
    )
      return null
    if (value.source !== source || (value.kind !== 'create' && (!value.target || typeof value.target.id !== 'string')))
      return null
    return value
  } catch {
    return null
  }
}
