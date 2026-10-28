import type { InlineDocument } from './model'
import { shallowRef, type ShallowRef } from 'vue'
import { reconcileUiState, type UiProgram, type UiState } from './program'

// Bounded, tab-local state. Scope includes server/session/reference/panel, never
// just a model-authored label. Invalid restored values are reconciled by schema.
const states = new Map<string, ShallowRef<UiState>>()
function storageKey(scope: string) {
  return `agena.inline-ui.state.v1:${scope}`
}
export function programState(scope: string, program: UiProgram): ShallowRef<UiState> {
  let state = states.get(scope)
  if (!state) {
    let stored: UiState = {}
    try {
      const raw = sessionStorage.getItem(storageKey(scope))
      const parsed: unknown = JSON.parse(raw && raw.length <= 256 * 1024 ? raw : '{}')
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) stored = parsed as UiState
    } catch {
      /* storage is optional */
    }
    state = shallowRef(reconcileUiState(program, stored))
    states.set(scope, state)
    while (states.size > 64) states.delete(states.keys().next().value!)
  }
  return state
}
export function saveProgramState(scope: string, state: UiState) {
  const current = states.get(scope)
  if (current) current.value = state
  try {
    sessionStorage.setItem(storageKey(scope), JSON.stringify(state))
  } catch {
    /* quota/private mode does not break controls */
  }
}
export function readProgramStates(scope: string, document?: InlineDocument): Record<string, UiState> {
  const result: Record<string, UiState> = Object.create(null)
  for (const [key, value] of states) if (key.startsWith(scope + '/')) result[key.slice(scope.length + 1)] = value.value
  for (const block of document?.blocks || [])
    if (block.type === 'panel') result[block.id] = programState(`${scope}/${block.id}`, block.program).value
  return result
}
